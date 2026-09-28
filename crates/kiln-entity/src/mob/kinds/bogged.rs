//! Bogged: the swamp skeleton, 16 health, slower to shoot (every 70 ticks, 50 on hard) and with
//! poison on its arrows; shears take its mushrooms.

use super::skeleton;
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityLevel, Event};
use crate::math::Vec3;
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, Info, Kind, MobExt};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, GroupData, MobData, SpawnContext};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Bogged;

pub static KIND: Bogged = Bogged;

static INFO: Info = Info {
    burns_in_daylight: true,
    breathes_under_water: true,
    ..Info::monster("minecraft:bogged", &[(MovementSpeed, 0.25), (MaxHealth, 16.0)])
};

#[derive(Clone, Debug, Default)]
pub struct State {
    /// `DATA_SHEARED`.
    pub sheared: bool,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("bogged state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("bogged state")
}

impl Kind for Bogged {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(State::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        skeleton::register_goals(m);
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, _group: &mut GroupData) {
        skeleton::finalize(e, m, r, ctx);
    }

    /// `getAttackInterval` / `getHardAttackInterval`.
    fn bow_interval(&self, hard: bool) -> Option<i32> {
        Some(if hard { 50 } else { 70 })
    }

    /// `Bogged.getArrow`: a plain arrow carries poison (5 seconds; an eighth of it on a hit).
    fn ranged_arrow(&self, _e: &mut Entity, _m: &mut MobData, arrow: &mut Entity) {
        if arrow.type_name == "minecraft:arrow"
            && let EntityKind::Arrow(a) = &mut arrow.kind
        {
            a.effects.push(("minecraft:poison", 100, 0));
        }
    }

    /// Shears take the mushrooms (dropped at the top of the head).
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        if stack.is_empty() || mob::item_name(stack) != "minecraft:shears" || st(m).sheared {
            return None;
        }
        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.bogged.shear", source: "players", volume: 1.0, pitch: 1.0 });
        let p = e.position();
        level.emit(Event::ShearLoot { entity: e.id, table: "minecraft:shearing/bogged".into(), pos: Vec3::new(p.x, p.y + e.height as f64 - 1.0, p.z) });
        st_mut(m).sheared = true;
        level.emit(Event::GameEvent { event: "minecraft:shear", pos: e.position(), entity: Some(who.id) });
        Some(Outcome::success(HeldChange::Damage(1)))
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let sheared = r.bool_or("sheared", false);
        st_mut(m).sheared = sheared;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        o.put("sheared", Tag::Byte(st(m).sheared as i8));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(kiln_data::entities::data::bogged::SHEARED, &DataValue::Boolean(st(m).sheared));
    }
}
