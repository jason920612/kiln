//! Interaction entities (`Interaction`): an invisible box of a set size that remembers who last hit it and who
//! last right clicked it (for datapacks: `execute on attacker`, `execute on target`, `data get entity`). With
//! `response` on, the hit and the click are answered like those on any entity (the arm swings).

use crate::entity::Entity;
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityLevel};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::persist::{Input, Output, uuid_from_tag, uuid_to_tag};
use kiln_item::ItemStack;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

/// `Interaction$PlayerAction`: a player (by UUID) and the game time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerAction {
    pub player: u128,
    pub timestamp: i64,
}

impl PlayerAction {
    fn read(tag: &Tag) -> Option<PlayerAction> {
        Some(PlayerAction { player: uuid_from_tag(tag.get("player")?)?, timestamp: tag.get("timestamp")?.as_i64()? })
    }

    fn write(&self) -> Tag {
        Tag::Compound(vec![("player".into(), uuid_to_tag(self.player)), ("timestamp".into(), Tag::Long(self.timestamp))])
    }
}

#[derive(Clone, Debug)]
pub struct Interaction {
    pub width: f32,
    pub height: f32,
    pub response: bool,
    pub attack: Option<PlayerAction>,
    pub interaction: Option<PlayerAction>,
}

pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    Some(Box::new(Interaction {
        width: r.float_or("width", 1.0),
        height: r.float_or("height", 1.0),
        attack: r.get("attack").and_then(PlayerAction::read),
        interaction: r.get("interaction").and_then(PlayerAction::read),
        response: r.bool_or("response", false),
    }))
}

/// After the entity's position is read: its box is the saved size around it (`makeBoundingBox`).
pub fn after_load(e: &mut Entity) {
    let Some(i) = crate::ext_entity::get::<Interaction>(e) else { return };
    let (w, h) = (i.width, i.height);
    e.width = w;
    e.height = h;
    let p = e.position();
    e.set_pos(p);
}

impl EntityExt for Interaction {
    crate::entity_ext_boilerplate!();

    fn tick(&mut self, _e: &mut Entity, _level: &mut dyn EntityLevel) {}

    fn save(&self, _e: &Entity, o: &mut Output) {
        o.put("width", Tag::Float(self.width));
        o.put("height", Tag::Float(self.height));
        if let Some(a) = &self.attack {
            o.put("attack", a.write());
        }
        if let Some(a) = &self.interaction {
            o.put("interaction", a.write());
        }
        o.put("response", Tag::Byte(self.response as i8));
    }

    fn entity_data(&self, _e: &Entity, d: &mut EntityData) {
        use kiln_data::entities::data::interaction;
        if self.width != 1.0 {
            d.set(interaction::WIDTH, &DataValue::Float(self.width));
        }
        if self.height != 1.0 {
            d.set(interaction::HEIGHT, &DataValue::Float(self.height));
        }
        if self.response {
            d.set(interaction::RESPONSE, &DataValue::Boolean(true));
        }
    }

    /// `isAttackable` is true: a player's hit gets to `skipAttackInteraction`.
    fn attackable(&self) -> bool {
        true
    }

    /// `skipAttackInteraction(attacker)` (the hit with no damage, as hanging entities get theirs): the attacking
    /// player is remembered; with `response` the hit goes on as any, else it ends here. `hurtServer` itself is
    /// always false.
    fn hurt(&mut self, _e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, amount: f32, attacker: Option<i32>) -> bool {
        if kind != DamageKind::PlayerAttack || amount != 0.0 {
            return false;
        }
        let Some(p) = attacker.and_then(|a| level.player(a)) else { return false };
        self.attack = Some(PlayerAction { player: p.uuid, timestamp: level.game_time() });
        !self.response
    }

    /// `interact`: the player is remembered; the click is taken (no swing: `CONSUME`).
    fn interact(&mut self, _e: &mut Entity, level: &mut dyn EntityLevel, who: &Interactor, _stack: &ItemStack) -> Option<Outcome> {
        let uuid = level.player(who.id)?.uuid;
        self.interaction = Some(PlayerAction { player: uuid, timestamp: level.game_time() });
        Some(Outcome { success: true, held: HeldChange::None, shear: None, player_sound: None, ride: false, open_container: false, sheared: None })
    }
}
