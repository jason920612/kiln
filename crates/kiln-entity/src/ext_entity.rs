//! Entity types (mostly projectiles) that bring their behaviour in a module of their own:
//! [`EntityKind::Ext`](crate::EntityKind::Ext) holds an [`EntityExt`].

use crate::entity::{Entity, EntityKind};
use crate::level::{DamageKind, EntityLevel};
use crate::persist::{Input, Output};
use kiln_proto::packets::entity::EntityData;
use std::any::Any;
use std::fmt::Debug;

pub mod area_effect_cloud;
pub mod fireball;
pub mod fishing_hook;
pub mod lightning;
pub mod shulker_bullet;
pub mod trident;

/// An extension entity's state and behaviour.
pub trait EntityExt: Any + Debug + Send + Sync {
    fn box_clone(&self) -> Box<dyn EntityExt>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// One server tick (`Entity.tick`); the entity's kind is a stand-in meanwhile.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel);
    /// `getDefaultGravity`.
    fn gravity(&self) -> f64 {
        0.0
    }
    /// `addAdditionalSaveData`.
    fn save(&self, e: &Entity, o: &mut Output) {
        let _ = (e, o);
    }
    /// Entity data for viewers.
    fn entity_data(&self, e: &Entity, d: &mut EntityData) {
        let _ = (e, d);
    }
    /// The spawn packet's data field (`getAddEntityPacket`: often the owner's id).
    fn spawn_data(&self) -> i32 {
        0
    }
    /// `hurtServer`: whether the hit did something (a deflected fireball).
    fn hurt(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, amount: f32, attacker: Option<i32>) -> bool {
        let _ = (e, level, kind, amount, attacker);
        false
    }
}

impl Clone for Box<dyn EntityExt> {
    fn clone(&self) -> Self {
        EntityExt::box_clone(&**self)
    }
}

/// Implements `box_clone` and `as_any` of [`EntityExt`] for a `Clone` type.
#[macro_export]
macro_rules! entity_ext_boilerplate {
    () => {
        fn box_clone(&self) -> Box<dyn $crate::ext_entity::EntityExt> {
            Box::new(self.clone())
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    };
}

/// The extension entity types.
pub const TYPES: &[&str] = &["minecraft:trident", "minecraft:fireball", "minecraft:small_fireball", "minecraft:shulker_bullet", "minecraft:area_effect_cloud"];

/// Reads a saved extension entity (`None`: not one of these types, or not simulated yet).
pub fn load(type_name: &'static str, r: &mut Input) -> Option<Box<dyn EntityExt>> {
    match type_name {
        "minecraft:trident" => trident::load(r),
        "minecraft:fireball" | "minecraft:small_fireball" => fireball::load(type_name, r),
        "minecraft:shulker_bullet" => shulker_bullet::load(r),
        "minecraft:area_effect_cloud" => area_effect_cloud::load(r),
        _ => None,
    }
}

/// The extension state of `e` as `T`.
pub fn get<T: 'static>(e: &Entity) -> Option<&T> {
    match &e.kind {
        EntityKind::Ext(x) => x.as_any().downcast_ref::<T>(),
        _ => None,
    }
}

pub fn get_mut<T: 'static>(e: &mut Entity) -> Option<&mut T> {
    match &mut e.kind {
        EntityKind::Ext(x) => x.as_any_mut().downcast_mut::<T>(),
        _ => None,
    }
}

/// `Entity.tick` for an extension entity.
pub(crate) fn tick(e: &mut Entity, level: &mut dyn EntityLevel) {
    let placeholder = EntityKind::Other { type_name: e.type_name };
    let EntityKind::Ext(mut x) = std::mem::replace(&mut e.kind, placeholder) else { return };
    x.tick(e, level);
    if matches!(e.kind, EntityKind::Other { .. }) {
        e.kind = EntityKind::Ext(x);
    }
}
