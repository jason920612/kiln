//! Leash knots (`LeashFenceKnotEntity`): the entity a lead is tied to on a fence. It sits on
//! the block (0.5 over its centre, 0.375 up), checks every 100 ticks that the block is still a
//! fence, goes away when the last lead is taken off it, and is destroyed by a hit.

use crate::blocks::{Tag as BlockTag, has_tag};
use crate::entity::{Entity, EntityKind, RemovalReason};
use crate::ext_entity::EntityExt;
use crate::leash;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::persist::{Input, Output};
use kiln_proto::nbt::Tag;

#[derive(Clone, Debug)]
pub struct LeashKnot {
    /// `BlockAttachedEntity.ticksSinceLastCheck`.
    pub since_check: i32,
}

/// `new LeashFenceKnotEntity(level, pos)` (`recalculateBoundingBox` puts it on the block).
pub fn new(id: i32, pos: BlockPos, seed: i64) -> Entity {
    let mut e = Entity::new(leash::KNOT, id, 0, EntityKind::Other { type_name: leash::KNOT }, seed);
    e.kind = EntityKind::Ext(Box::new(LeashKnot { since_check: 0 }));
    e.set_pos(Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.375, pos.z as f64 + 0.5));
    e.set_old_pos_and_rot();
    e
}

pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    // (`block_pos` is the saved position's block: `Pos` carries the same.)
    let _ = r.get("block_pos");
    Some(Box::new(LeashKnot { since_check: 0 }))
}

/// `BlockAttachedEntity.getPos`.
fn block_of(e: &Entity) -> BlockPos {
    let p = e.position();
    BlockPos::containing(p.x, p.y, p.z)
}

impl EntityExt for LeashKnot {
    crate::entity_ext_boilerplate!();

    /// `BlockAttachedEntity.tick`: every 100 ticks the fence must still be there.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        self.since_check += 1;
        if self.since_check >= 100 {
            self.since_check = 0;
            if !e.is_removed() && !has_tag(level.block(block_of(e)), BlockTag::Fences) {
                e.discard();
                drop_item(e, level);
            }
        }
    }

    fn save(&self, e: &Entity, o: &mut Output) {
        let b = block_of(e);
        o.put("block_pos", Tag::IntArray(vec![b.x, b.y, b.z]));
    }

    /// A hit (of a player: `skipAttackInteraction`) kills the knot and drops what it was.
    fn hurt(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, _amount: f32, attacker: Option<i32>) -> bool {
        if e.is_invulnerable_to_base(kind) {
            return false;
        }
        if !level.mob_griefing() && attacker.and_then(|a| level.entity(a)).is_some_and(|a| crate::mob::data(a).is_some()) {
            return false;
        }
        if !e.is_removed() {
            e.removed = Some(RemovalReason::Killed);
            level.emit(Event::GameEvent { event: "minecraft:entity_die", pos: e.position(), entity: attacker });
            drop_item(e, level);
        }
        true
    }

    fn attackable(&self) -> bool {
        true
    }

    /// `BlockAttachedEntity.thunderHit`: nothing.
    fn thunder_hit(&mut self, _e: &mut Entity, _level: &mut dyn EntityLevel, _bolt: i32) -> bool {
        true
    }

    /// `LeashFenceKnotEntity.interact`: shears cut every lead on the knot; otherwise the leads
    /// the player holds go on the knot, and when it holds none of them the leads on the knot go
    /// to the player (not when sneaking).
    fn interact(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, who: &Interactor, stack: &kiln_item::ItemStack) -> Option<Outcome> {
        if !stack.is_empty() && crate::mob::item_name(stack) == "minecraft:shears" && leash::shear_off_all(e, level, who.id) {
            return Some(Outcome::success(HeldChange::Damage(1)));
        }
        let mut any = false;
        let centre = level.entity(who.id).map_or(e.position(), |p| p.bounding_box().center());
        for id in leash::leashed_to(level, who.id, centre) {
            if level.entity(id).is_some_and(|l| leash::can_have_leash_attached_to(l, None, e, &*level)) {
                leash::set_leashed_to_in_level(level, id, e.id);
                any = true;
            }
        }
        let mut any2 = false;
        if !any && !who.sneaking {
            for id in leash::leashed_to(level, e.id, e.bounding_box().center()) {
                let ok = level.entity(who.id).is_some_and(|p| level.entity(id).is_some_and(|l| leash::can_have_leash_attached_to(l, None, p, &*level)));
                if ok {
                    leash::set_leashed_to_in_level(level, id, who.id);
                    any2 = true;
                }
            }
        }
        if any || any2 {
            // `notifyLeasheeRemoved` for the leads that went to the player: the knot is out of the
            // level while it is clicked.
            if any2 && leash::leashed_to(level, e.id, e.bounding_box().center()).is_empty() {
                e.discard();
            }
            level.emit(Event::GameEvent { event: "minecraft:block_attach", pos: e.position(), entity: Some(who.id) });
            level.emit(Event::Sound { pos: e.position(), sound: "minecraft:item.lead.tied", source: "neutral", volume: 1.0, pitch: 1.0 });
            return Some(Outcome::success(HeldChange::None));
        }
        None
    }
}

/// `LeashFenceKnotEntity.dropItem`: only the sound of the lead coming off.
fn drop_item(e: &Entity, level: &mut dyn EntityLevel) {
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:item.lead.untied", source: "neutral", volume: 1.0, pitch: 1.0 });
    }
}
