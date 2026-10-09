//! Pig (`Pig`): the farm animal (goals, breeding, the climate variants and the lightning
//! conversion live in the shared mob code) that takes a saddle and is then ridden, steered with a
//! carrot on a stick, which boosts it when used.

use super::steering::{Saddle, Steering};
use crate::entity::Entity;
use crate::level::{EntityLevel, PlayerView};
use crate::math::Vec3;
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, Info, Kind, MobExt};
use crate::mob::goals::Goal;
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, MobData, item_tag};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Pig;

pub static KIND: Pig = Pig;

static INFO: Info = Info::animal("minecraft:pig", &[(MaxHealth, 10.0), (MovementSpeed, 0.25)]);

const CARROT_STICK: &str = "minecraft:carrot_on_a_stick";

#[derive(Clone, Debug, Default)]
pub struct State {
    pub saddle: Saddle,
    pub steering: Steering,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("pig state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("pig state")
}

impl Kind for Pig {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(State::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        let tempt = || Goal::Tempt { speed: 1.2, calm_down: 0, player: None };
        g.add(0, Goal::Float);
        g.add(1, Goal::Panic { speed: 1.25, pos: Vec3::ZERO });
        g.add(3, Goal::Breed { speed: 1.0, partner: None, love_time: 0 });
        g.add(4, tempt());
        g.add(4, tempt());
        g.add(5, Goal::FollowParent { speed: 1.1, parent: None, recalc: 0 });
        g.add(6, Goal::RandomStroll { speed: 1.0, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false });
        g.add(7, Goal::LookAtPlayer { dist: 6.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
    }

    fn is_food(&self, item: i32) -> bool {
        item_tag(item, "minecraft:pig_food")
    }

    /// `Pig.getControllingPassenger`: a saddled pig is steered by a rider with a carrot on a stick.
    fn steerable_by(&self, m: &MobData, rider: &PlayerView) -> bool {
        let stick = kiln_data::builtin_id("minecraft:item", CARROT_STICK);
        st(m).saddle.is_saddled() && (Some(rider.main_hand) == stick || Some(rider.off_hand) == stick)
    }

    /// `Pig.tickRidden`: the rider turns the pig, and the boost runs down.
    fn tick_ridden(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, rider: &PlayerView) {
        e.y_rot = rider.yaw % 360.0;
        e.x_rot = (rider.pitch * 0.5) % 360.0;
        e.y_rot_o = e.y_rot;
        m.y_body_rot = e.y_rot;
        m.y_head_rot = e.y_rot;
        st_mut(m).steering.tick_boost();
    }

    fn stick(&self) -> Option<(&'static str, i32)> {
        Some((CARROT_STICK, 7))
    }

    fn boost(&self, e: &mut Entity, m: &mut MobData) -> bool {
        st_mut(m).steering.boost(&mut e.random)
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        let food = !stack.is_empty() && self.is_food(stack.item());
        if !food && st(m).saddle.is_saddled() && e.passengers.is_empty() && !who.sneaking {
            let mut out = Outcome::success(HeldChange::None);
            out.ride = true;
            return Some(out);
        }
        let out = mob::interact::animal_interact(e, m, level, who, stack);
        if out.success {
            return Some(out);
        }
        // `isEquippableInSlot(stack, SADDLE)` and `interactLivingEntity`: `Equippable.equipOnTarget`.
        if !m.baby() && !st(m).saddle.is_saddled() && mob::is_alive(e, m)
            && let Some(one) = super::steering::equip_on_target(e, level, stack, EquipmentSlot::Saddle, Some("minecraft:entity.pig.saddle"))
        {
            st_mut(m).saddle.put_guaranteed(one);
            // `ItemStack.split(1)`, whatever the game mode.
            return Some(Outcome::success(HeldChange::Shrink(1)));
        }
        Some(Outcome::PASS)
    }

    fn set_extra_equipment(&self, m: &mut MobData, slot: u8, stack: ItemStack) -> bool {
        if slot != 7 {
            return false;
        }
        st_mut(m).saddle.put_guaranteed(stack);
        true
    }

    fn extra_equipment(&self, m: &MobData) -> Vec<(u8, ItemStack)> {
        let s = &st(m).saddle;
        if s.is_saddled() { vec![(7, s.stack.clone())] } else { Vec::new() }
    }

    fn take_extra_equipment_for_drop(&self, m: &mut MobData) -> Vec<(ItemStack, f32)> {
        let s = &mut st_mut(m).saddle;
        vec![(std::mem::take(&mut s.stack), s.drop)]
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        st_mut(m).saddle.load(r);
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        st(m).saddle.save(o);
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        if s.steering.total != 0 {
            d.set(data::pig::BOOST_TIME, &DataValue::Int(s.steering.total));
        }
        d.set(data::pig::VARIANT, &DataValue::Holder(m.variant));
        d.set(data::pig::SOUND_VARIANT, &DataValue::Holder(m.sound_variant));
    }
}
