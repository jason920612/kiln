//! Wither skulls (`AbstractHurtingProjectile` -> `WitherSkull`): the wither's heads shoot them.
//! They fly straight on like fireballs (the blue, dangerous ones slow down faster), never burn,
//! and on a hit deal 8 (the wither heals 5 if that kills) with wither II for 10 or 40 seconds
//! by difficulty, then explode with power 1. A dangerous skull's explosion treats every block
//! the wither can break as having a resistance of at most 0.8.

use super::fireball;
use crate::entity::{Entity, EntityKind};
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::Vec3;
use crate::mob::{self, DamageSource};
use crate::persist::{Input, Output};
use crate::projectile::Hit;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

#[derive(Clone, Debug)]
pub struct WitherSkull {
    pub owner: Option<i32>,
    pub dangerous: bool,
    pub acceleration_power: f64,
    pub left_owner: bool,
    pub has_been_shot: bool,
}

/// A new skull from `owner` (its feet, facing its rotation) heading along `dir`
/// (`new WitherSkull(level, mob, direction)`); the caller places it at the head.
pub fn new(id: i32, owner: &Entity, dir: Vec3, dangerous: bool, seed: i64) -> Entity {
    let x = WitherSkull { owner: Some(owner.id), dangerous, acceleration_power: 0.1, left_owner: false, has_been_shot: false };
    let mut e = Entity::new("minecraft:wither_skull", id, 0, EntityKind::Ext(Box::new(x)), seed);
    e.set_pos(owner.position());
    // `assignDirectionalMovement(direction, accelerationPower)`.
    e.delta = dir.normalize().scale(0.1);
    e.needs_sync = true;
    e.y_rot = owner.y_rot;
    e.x_rot = owner.x_rot;
    e.set_old_pos_and_rot();
    e
}

/// Reads a saved one.
pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    Some(Box::new(WitherSkull {
        owner: None,
        dangerous: r.bool_or("dangerous", false),
        acceleration_power: r.num("acceleration_power").unwrap_or(0.1),
        left_owner: r.bool_or("leftOwner", false),
        has_been_shot: r.bool_or("HasBeenShot", false),
    }))
}

/// `WitherBoss.canDestroy`: not air, not `#wither_immune`.
pub fn wither_can_destroy(state: u16) -> bool {
    !kiln_data::blocks_types::is_air(state) && !block_tag(state, "minecraft:wither_immune")
}

/// Whether block state `state` is in the `minecraft:block` tag `tag`.
pub fn block_tag(state: u16, tag: &str) -> bool {
    let Some(id) = kiln_data::builtin_id("minecraft:block", crate::blocks::block_name(state)) else { return false };
    kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == "minecraft:block")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .is_some_and(|(_, ids)| ids.contains(&id))
}

impl WitherSkull {
    fn on_hit(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, hit: Hit) {
        // `hitTargetOrDeflectSelf`: a breeze turns it back (`onDeflection(false)`: half the
        // acceleration) and nothing is hit.
        if let Hit::Entity { id, .. } = hit
            && let Some(turned) = crate::projectile::try_deflect(e, level, id)
        {
            if turned {
                self.acceleration_power *= 0.5;
            }
            return;
        }
        level.emit(Event::ProjectileHit { projectile: e.id, projectile_type: "minecraft:wither_skull", owner: self.owner, hit });
        if let Hit::Entity { id, .. } = hit {
            // `onHitEntity`: the owner's skull hits for 8, an ownerless one for 5 (magic).
            let owner = self.owner.and_then(|o| goals_living(level, o)).map(|_| self.owner.unwrap());
            let hurt = match owner {
                Some(o) => {
                    let source = DamageSource { kind: DamageKind::WitherSkull, attacker: Some(o), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
                    let hurt = fireball::hurt(level, id, source, 8.0);
                    if hurt && !goals_living(level, id).is_some_and(|t| t.alive) {
                        heal(level, o, 5.0);
                    }
                    hurt
                }
                None => {
                    let source = DamageSource { kind: DamageKind::Magic, attacker: None, direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
                    fireball::hurt(level, id, source, 5.0)
                }
            };
            if hurt && goals_living(level, id).is_some() {
                let seconds = match level.difficulty() {
                    2 => 10,
                    3 => 40,
                    _ => 0,
                };
                if seconds > 0 {
                    level.add_effect(id, "minecraft:wither", 20 * seconds, 1, Some(owner.unwrap_or(e.id)));
                }
            }
        }
        // `onHit`: the explosion (`ExplosionInteraction.MOB`).
        let griefing = level.mob_griefing();
        let interaction = if griefing { crate::explosion::Interaction::DestroyWithDecay } else { crate::explosion::Interaction::Keep };
        let resist = |state: u16, r: f32| if wither_can_destroy(state) { r.min(0.8) } else { r };
        let resistance: Option<crate::explosion::Resistance> = if self.dangerous { Some(&resist) } else { None };
        crate::explosion::explode_with(level, Some(e.id), e.position(), 1.0, false, interaction, resistance, true);
        e.discard();
    }
}

fn goals_living(level: &dyn EntityLevel, id: i32) -> Option<mob::goals::Living> {
    mob::goals::living(level, id)
}

/// `LivingEntity.heal` of mob `id`.
fn heal(level: &mut dyn EntityLevel, id: i32, amount: f32) {
    if let Some(m) = level.entity_mut(id).and_then(mob::data_mut)
        && m.health > 0.0
    {
        let h = m.health + amount;
        m.set_health(h);
    }
}

impl EntityExt for WitherSkull {
    crate::entity_ext_boilerplate!();

    /// `AbstractHurtingProjectile.tick`.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        // `applyInertia`: `getInertia` is 0.73 for a dangerous skull.
        let v = e.delta;
        let inertia = if e.is_in_water() {
            0.8f32
        } else if self.dangerous {
            0.73
        } else {
            0.95
        };
        e.delta = (v + v.normalize().scale(self.acceleration_power)).scale(inertia as f64);
        let owner_gone = self.owner.and_then(|o| level.entity(o)).is_some_and(|o| o.is_removed());
        if owner_gone || !level.is_loaded(e.block_position()) {
            e.discard();
            return;
        }
        let hit = fireball::hit_on_move_vector(e, level, self.owner, self.left_owner);
        let to = match hit {
            Some(Hit::Block { location, .. } | Hit::Entity { location, .. }) => location,
            None => e.position() + e.delta,
        };
        fireball::rotate_towards_movement(e, 0.2);
        e.set_pos(to);
        e.apply_effects_from_blocks(level);
        // `Projectile.tick`.
        if !self.has_been_shot {
            level.emit(Event::GameEvent { event: "minecraft:projectile_shoot", pos: e.position(), entity: self.owner });
            self.has_been_shot = true;
        }
        if !self.left_owner {
            self.left_owner = fireball::left_owner(e, level, self.owner);
        }
        e.base_tick(level);
        // `shouldBurn` is false: no fire.
        if let Some(hit) = hit
            && e.is_alive()
        {
            self.on_hit(e, level, hit);
        }
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        o.put("leftOwner", Tag::Byte(self.left_owner as i8));
        o.put("HasBeenShot", Tag::Byte(self.has_been_shot as i8));
        o.put("acceleration_power", Tag::Double(self.acceleration_power));
        o.put("dangerous", Tag::Byte(self.dangerous as i8));
    }

    fn entity_data(&self, _e: &Entity, d: &mut EntityData) {
        d.set(kiln_data::entities::data::wither_skull::DANGEROUS, &DataValue::Boolean(self.dangerous));
    }

    fn spawn_data(&self) -> i32 {
        self.owner.unwrap_or(0)
    }
}
