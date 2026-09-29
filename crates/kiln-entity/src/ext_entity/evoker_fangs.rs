//! Evoker fangs (`EvokerFangs`): after their warm-up they snap (entity event 4 for the
//! client's animation and sound), bite the living entities they touch for 6 (indirect magic
//! from their owner, sparing its allies) and vanish.

use crate::entity::{Entity, EntityKind};
use crate::entity_ext_boilerplate;
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::Vec3;
use crate::mob::DamageSource;
use crate::persist::{Input, Output};
use kiln_proto::nbt::Tag;

#[derive(Clone, Debug)]
pub struct EvokerFangs {
    pub warmup: i32,
    sent_spike_event: bool,
    life_ticks: i32,
    /// The evoker (network id while known) and its UUID.
    pub owner: Option<i32>,
    pub owner_uuid: Option<u128>,
}

/// Fangs at `pos` turned `angle` radians, snapping after `warmup` ticks.
pub fn new(id: i32, pos: Vec3, angle: f32, warmup: i32, owner: &Entity, seed: i64) -> Entity {
    let f = EvokerFangs { warmup, sent_spike_event: false, life_ticks: 22, owner: Some(owner.id), owner_uuid: Some(owner.uuid) };
    let mut e = Entity::new("minecraft:evoker_fangs", id, 0, EntityKind::Ext(Box::new(f)), seed);
    e.y_rot = angle * (180.0 / std::f32::consts::PI);
    e.set_pos(pos);
    e.set_old_pos_and_rot();
    e
}

pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let warmup = r.int_or("Warmup", 0);
    let owner_uuid = r.uuid("Owner");
    Some(Box::new(EvokerFangs { warmup, sent_spike_event: false, life_ticks: 22, owner: None, owner_uuid }))
}

/// `Mob.isAlliedTo` of an evoker (`considersEntityAsAlly`): itself, illager friends (no teams),
/// and vexes it (or an ally) summoned.
pub fn evoker_ally(level: &dyn EntityLevel, evoker: i32, other: i32) -> bool {
    if other == evoker {
        return true;
    }
    let Some(o) = level.entity(other) else { return false };
    if crate::mob::kinds::raider::illager_ally(o.type_name) {
        return true;
    }
    let owner = crate::mob::data(o).and_then(crate::mob::kinds::vex::owner);
    owner.is_some_and(|w| w == evoker || level.entity(w).is_some_and(|we| crate::mob::kinds::raider::illager_ally(we.type_name)))
}

impl EvokerFangs {
    /// `dealDamageTo`.
    fn deal_damage(&self, e: &Entity, level: &mut dyn EntityLevel, target: i32) {
        let owner = self.owner.filter(|&o| level.entity(o).is_some_and(|oe| oe.is_alive()));
        if Some(target) == owner {
            return;
        }
        let Some(t) = crate::mob::goals::living(level, target) else { return };
        if !t.alive || t.invulnerable {
            return;
        }
        let source = match owner {
            None => DamageSource::of(DamageKind::Magic),
            Some(o) => {
                if evoker_ally(level, o, target) {
                    return;
                }
                DamageSource { kind: DamageKind::IndirectMagic, attacker: Some(o), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false }
            }
        };
        crate::mob::hurt_living(level, &t, source, 6.0);
    }
}

impl EntityExt for EvokerFangs {
    entity_ext_boilerplate!();

    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        e.base_tick(level);
        self.warmup -= 1;
        if self.warmup >= 0 {
            return;
        }
        if self.warmup == -8 {
            let area = e.bounding_box().inflate(0.2, 0.0, 0.2);
            for id in level.entities_in(&area, EntityFilter::Living, e.id) {
                self.deal_damage(e, level, id);
            }
        }
        if !self.sent_spike_event {
            level.emit(Event::EntityEvent { entity: e.id, event: 4 });
            self.sent_spike_event = true;
        }
        self.life_ticks -= 1;
        if self.life_ticks < 0 {
            e.discard();
        }
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        o.put("Warmup", Tag::Int(self.warmup));
        if let Some(u) = self.owner_uuid {
            o.put("Owner", crate::persist::uuid_to_tag(u));
        }
    }

    fn hurt(&mut self, _e: &mut Entity, _level: &mut dyn EntityLevel, _kind: DamageKind, _amount: f32, _attacker: Option<i32>) -> bool {
        false
    }
}
