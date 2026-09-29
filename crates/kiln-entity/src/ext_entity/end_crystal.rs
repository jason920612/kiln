//! End crystals (`EndCrystal`): stand still, keep a fire burning under them in a level with a
//! dragon fight, heal the ender dragon (see [`crate::mob::kinds::ender_dragon`]) and explode
//! (power 6) when anything but the dragon hurts them, which the fight hears about. A crystal
//! may show a beam to a block (the respawn ritual, the pillars' guard).

use crate::entity::{Entity, EntityKind};
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, DragonFightEvent, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

#[derive(Clone, Debug)]
pub struct EndCrystal {
    /// `time`: the client's spin (random start).
    pub time: i32,
    pub beam_target: Option<BlockPos>,
    pub show_bottom: bool,
}

/// `new EndCrystal(level, x, y, z)` (`time` from the crystal's random).
pub fn new(id: i32, pos: Vec3, show_bottom: bool, seed: i64) -> Entity {
    let mut e = Entity::new("minecraft:end_crystal", id, 0, EntityKind::Other { type_name: "minecraft:end_crystal" }, seed);
    let time = e.random.next_int_bounded(100000);
    e.kind = EntityKind::Ext(Box::new(EndCrystal { time, beam_target: None, show_bottom }));
    e.set_pos(pos);
    e.set_old_pos_and_rot();
    e
}

pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let beam_target = match r.get("beam_target") {
        Some(Tag::IntArray(v)) if v.len() == 3 => Some(BlockPos::new(v[0], v[1], v[2])),
        _ => None,
    };
    Some(Box::new(EndCrystal { time: 0, beam_target, show_bottom: r.bool_or("ShowBottom", true) }))
}

/// The crystal state of `e`, if it is an end crystal.
pub fn get_mut(e: &mut Entity) -> Option<&mut EndCrystal> {
    crate::ext_entity::get_mut::<EndCrystal>(e)
}

impl EntityExt for EndCrystal {
    crate::entity_ext_boilerplate!();

    /// `EndCrystal.tick`: fire under it (in a level with a dragon fight) when the block is air.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        self.time += 1;
        e.apply_effects_from_blocks(level);
        let p = e.block_position();
        if level.dragon_fight().is_some() && crate::physics::is_air(level.block(p)) {
            let fire = crate::ext_entity::fireball::fire_state(level, p);
            level.set_block(p, fire, 3);
        }
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        if let Some(b) = self.beam_target {
            o.put("beam_target", Tag::IntArray(vec![b.x, b.y, b.z]));
        }
        o.put("ShowBottom", Tag::Byte(self.show_bottom as i8));
    }

    fn entity_data(&self, _e: &Entity, d: &mut EntityData) {
        let b = self.beam_target.map(|b| [b.x, b.y, b.z]);
        d.set(kiln_data::entities::data::end_crystal::BEAM_TARGET, &DataValue::OptionalBlockPos(b));
        d.set(kiln_data::entities::data::end_crystal::SHOW_BOTTOM, &DataValue::Boolean(self.show_bottom));
    }

    /// `hurtServer`: not by its dragon; otherwise it is gone, exploding (power 6, the attacker
    /// credited) unless an explosion did it, and the fight hears (`onDestroyedBy`).
    fn hurt(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, amount: f32, attacker: Option<i32>) -> bool {
        let _ = amount;
        if e.is_invulnerable_to_base(kind) {
            return false;
        }
        if attacker.and_then(|a| level.entity(a)).is_some_and(|a| a.type_name == "minecraft:ender_dragon") {
            return false;
        }
        if !e.is_removed() {
            e.removed = Some(crate::entity::RemovalReason::Killed);
            if !kind.is_tag("minecraft:is_explosion") {
                let griefing_decay = crate::explosion::Interaction::DestroyWithDecay;
                crate::explosion::explode(level, Some(e.id), e.position(), 6.0, false, griefing_decay);
            }
            level.emit(Event::DragonFight(DragonFightEvent::CrystalDestroyed { crystal: e.id, uuid: e.uuid, pos: e.position(), kind, attacker }));
        }
        true
    }
}
