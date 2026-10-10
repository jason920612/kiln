//! Entity types (mostly projectiles) that bring their behaviour in a module of their own:
//! [`EntityKind::Ext`](crate::EntityKind::Ext) holds an [`EntityExt`].

use crate::entity::{Entity, EntityKind};
use crate::level::{DamageKind, EntityLevel};
use crate::persist::{Input, Output};
use kiln_proto::packets::entity::EntityData;
use std::any::Any;
use std::fmt::Debug;

pub mod boat;
pub mod cushion;
pub mod area_effect_cloud;
pub mod armor_stand;
pub mod display;
pub mod interaction;
pub mod marker;
pub mod ominous_item_spawner;
pub mod evoker_fangs;
pub mod dragon_fireball;
pub mod end_crystal;
pub mod eye_of_ender;
pub mod fireball;
pub mod minecart;
pub mod firework;
pub mod fishing_hook;
pub mod hanging;
pub mod item_frame;
pub mod painting;
pub mod painting_variants;
pub mod leash_knot;
pub mod lightning;
pub mod llama_spit;
pub mod shulker_bullet;
pub mod trident;
pub mod wither_skull;
pub mod wind_charge;

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
    /// What a viewer sees it wear: (equipment slot id, stack), the filled slots.
    fn equipment_shown(&self) -> Vec<(u8, kiln_item::ItemStack)> {
        Vec::new()
    }

    fn spawn_data(&self) -> i32 {
        0
    }
    /// `Entity.interact` (a right click): `Some` when the entity reacts (a boat takes the rider).
    fn interact(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, who: &crate::mob::interact::Interactor, stack: &kiln_item::ItemStack) -> Option<crate::mob::interact::Outcome> {
        let _ = (e, level, who, stack);
        None
    }
    /// Whether a player's melee hit reaches `hurt` (`isAttackable`: boats yes, fireballs no).
    fn attackable(&self) -> bool {
        false
    }
    /// `getPassengerAttachmentPoint` for the passenger at `index` (`None`: on top of the box).
    fn passenger_offset(&self, e: &Entity, index: usize, animal: bool) -> Option<crate::math::Vec3> {
        let _ = (e, index, animal);
        None
    }
    /// The slots of a container entity (`ContainerEntity`: chest and hopper minecarts, chest boats).
    fn container(&self) -> Option<&minecart::Contents> {
        None
    }
    fn container_mut(&mut self) -> Option<&mut minecart::Contents> {
        None
    }
    /// `Projectile.deflect(ProjectileDeflection.AIM_DEFLECT, by, owner = by, byAttack = true, 1.0)`
    /// as `Player.deflectProjectile` does for a projectile of `#minecraft:redirectable_projectile`:
    /// it flies on along `look` (the player's) with `by` (network id, UUID) as its owner. False for the rest.
    fn aim_deflect(&mut self, e: &mut Entity, by: (i32, u128), look: crate::math::Vec3) -> bool {
        let _ = (e, by, look);
        false
    }
    /// `Entity.thunderHit` of an entity that does its own (a block-attached entity ignores lightning, a cushion breaks): true
    /// when handled, so the plain fire and lightning damage does not follow.
    fn thunder_hit(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, bolt: i32) -> bool {
        let _ = (e, level, bolt);
        false
    }
    /// `hurtServer`: whether the hit did something.
    fn hurt(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, amount: f32, attacker: Option<i32>) -> bool {
        let _ = (e, level, kind, amount, attacker);
        false
    }
    /// `hurtServer` of a hit by a projectile whose state the entity may care about (a TNT
    /// minecart goes off when a burning arrow hits it); `on_fire` and `speed_sqr` are the
    /// projectile's `isOnFire` and `getDeltaMovement().lengthSqr()`.
    fn hurt_by_projectile(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, amount: f32, attacker: Option<i32>, on_fire: bool, speed_sqr: f64) -> bool {
        let _ = (on_fire, speed_sqr);
        self.hurt(e, level, kind, amount, attacker)
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
pub const TYPES: &[&str] = &[
    "minecraft:trident",
    "minecraft:fireball",
    "minecraft:small_fireball",
    "minecraft:shulker_bullet",
    "minecraft:area_effect_cloud",
    // -- slice 3: raids
    "minecraft:evoker_fangs",
    "minecraft:end_crystal",
    "minecraft:dragon_fireball",
    "minecraft:wither_skull",
    "minecraft:breeze_wind_charge",
    "minecraft:wind_charge",
    "minecraft:firework_rocket",
    // -- wp30: llamas
    "minecraft:llama_spit",
    // -- wp32: leads
    "minecraft:leash_knot",
    // -- wp44: the End
    "minecraft:eye_of_ender",
    // -- wp49: hanging entities
    "minecraft:item_frame",
    "minecraft:glow_item_frame",
    "minecraft:painting",
    "minecraft:armor_stand",
    // -- wp49: data entities
    "minecraft:block_display",
    "minecraft:item_display",
    "minecraft:text_display",
    "minecraft:interaction",
    "minecraft:marker",
    // -- wp50
    "minecraft:ominous_item_spawner",
    "minecraft:cushion",
];

/// Reads a saved extension entity (`None`: not one of these types, or not simulated yet).
pub fn load(type_name: &'static str, r: &mut Input) -> Option<Box<dyn EntityExt>> {
    match type_name {
        "minecraft:trident" => trident::load(r),
        "minecraft:fireball" | "minecraft:small_fireball" => fireball::load(type_name, r),
        "minecraft:shulker_bullet" => shulker_bullet::load(r),
        "minecraft:firework_rocket" => firework::load(r),
        n if boat::is_boat(n) => boat::load(n, r),
        n if minecart::is_minecart(n) => minecart::load(n, r),
        "minecraft:breeze_wind_charge" | "minecraft:wind_charge" => wind_charge::load(type_name, r),
        "minecraft:area_effect_cloud" => area_effect_cloud::load(r),
        "minecraft:evoker_fangs" => evoker_fangs::load(r),
        "minecraft:end_crystal" => end_crystal::load(r),
        "minecraft:dragon_fireball" => dragon_fireball::load(r),
        "minecraft:wither_skull" => wither_skull::load(r),
        "minecraft:llama_spit" => llama_spit::load(r),
        "minecraft:leash_knot" => leash_knot::load(r),
        "minecraft:eye_of_ender" => eye_of_ender::load(r),
        "minecraft:item_frame" => item_frame::load(false, r),
        "minecraft:glow_item_frame" => item_frame::load(true, r),
        "minecraft:painting" => painting::load(r),
        "minecraft:armor_stand" => armor_stand::load(r),
        n if display::is_display(n) => display::load(n, r),
        "minecraft:interaction" => interaction::load(r),
        "minecraft:marker" => marker::load(r),
        "minecraft:ominous_item_spawner" => ominous_item_spawner::load(r),
        "minecraft:cushion" => cushion::load(r),
        _ => None,
    }
}

/// `Entity.getPickResult`: what a middle click on `e` gives (`None`: nothing). A mob stands for its spawn egg; a few things for
/// their item (the armor stand, the end crystal, the painting, the lead of a knot, boats and minecarts, a cushion by its colour); an
/// item frame for what it holds, or itself.
pub fn pick_result(e: &Entity) -> Option<kiln_item::ItemStack> {
    use kiln_item::ItemStack;
    if let Some(m) = crate::mob::data(e)
        && m.kind.is_mob()
    {
        return ItemStack::of(&format!("{}_spawn_egg", e.type_name), 1);
    }
    let item = |name: &str| ItemStack::of(name, 1);
    match e.type_name {
        "minecraft:armor_stand" | "minecraft:end_crystal" | "minecraft:painting" => item(e.type_name),
        "minecraft:leash_knot" => item("minecraft:lead"),
        "minecraft:item_frame" | "minecraft:glow_item_frame" => {
            let frame = get::<item_frame::ItemFrame>(e)?;
            if frame.item.is_empty() { Some(frame.frame_item(e)) } else { Some(frame.item.clone()) }
        }
        "minecraft:cushion" => item(&cushion::item_name(get::<cushion::Cushion>(e)?.color)),
        // (A spawner minecart is a plain one in the hand.)
        "minecraft:spawner_minecart" => item("minecraft:minecart"),
        n if minecart::is_minecart(n) => item(n),
        n if n.ends_with("_boat") || n.ends_with("_raft") => item(n),
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

/// The slots of `e` when it is a container entity (chest or hopper minecart, chest boat).
pub fn container(e: &Entity) -> Option<&minecart::Contents> {
    match &e.kind {
        EntityKind::Ext(x) => x.container(),
        _ => None,
    }
}

pub fn container_mut(e: &mut Entity) -> Option<&mut minecart::Contents> {
    match &mut e.kind {
        EntityKind::Ext(x) => x.container_mut(),
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
