//! The extension mob types, one module each (see [`super::ext`]).

use super::MobKind;
use super::ext::Kind;

pub mod zombie;
pub mod skeleton;
pub mod husk;
pub mod stray;
pub mod drowned;
pub mod zombie_villager;
pub mod zombified_piglin;
pub mod wither_skeleton;
pub mod enderman;
pub mod endermite;
pub mod shulker;
pub mod witch;
pub mod slime;
pub mod magma_cube;
pub mod phantom;
pub mod ghast;
pub mod blaze;
pub mod anger;
pub mod tame;
pub mod wolf;
pub mod cat;
pub mod horse;
pub mod donkey;
pub mod mule;
pub mod strider;
pub mod iron_golem;
pub mod villager;
pub mod piglin;
pub mod hoglin;
// -- slice 3: raids

// -- slice 3: the end

// -- slice 3: wither and guardians

// -- slice 3: warden

// -- slice 3: common mobs A
pub mod common_a;
pub mod rabbit;
pub mod polar_bear;
pub mod turtle;
pub mod fox;

// -- slice 3: common mobs B
pub mod squid;
pub mod fish;
pub mod mooshroom;
pub mod ocelot;
pub mod bat;
pub mod snow_golem;
pub mod bogged;
pub mod armadillo;
pub mod camel;
pub mod allay;


/// The behaviour of an extension type; `None` for the shared-code types.
pub fn of(kind: MobKind) -> Option<&'static dyn Kind> {
    Some(match kind {
        MobKind::Husk => &husk::KIND,
        MobKind::Stray => &stray::KIND,
        MobKind::Drowned => &drowned::KIND,
        MobKind::ZombieVillager => &zombie_villager::KIND,
        MobKind::ZombifiedPiglin => &zombified_piglin::KIND,
        MobKind::WitherSkeleton => &wither_skeleton::KIND,
        MobKind::Enderman => &enderman::KIND,
        MobKind::Endermite => &endermite::KIND,
        MobKind::Shulker => &shulker::KIND,
        MobKind::Witch => &witch::KIND,
        MobKind::Slime => &slime::KIND,
        MobKind::MagmaCube => &magma_cube::KIND,
        MobKind::Phantom => &phantom::KIND,
        MobKind::Ghast => &ghast::KIND,
        MobKind::Blaze => &blaze::KIND,
        MobKind::Wolf => &wolf::KIND,
        MobKind::Cat => &cat::KIND,
        MobKind::Horse => &horse::KIND,
        MobKind::Donkey => &donkey::KIND,
        MobKind::Mule => &mule::KIND,
        MobKind::Strider => &strider::KIND,
        MobKind::IronGolem => &iron_golem::KIND,
        MobKind::Villager => &villager::KIND,
        MobKind::Piglin => &piglin::KIND,
        MobKind::Hoglin => &hoglin::KIND,
        // -- slice 3: raids

        // -- slice 3: the end

        // -- slice 3: wither and guardians

        // -- slice 3: warden

        // -- slice 3: common mobs A
        MobKind::Rabbit => &rabbit::KIND,
        MobKind::PolarBear => &polar_bear::KIND,
        MobKind::Turtle => &turtle::KIND,
        MobKind::Fox => &fox::KIND,

        // -- slice 3: common mobs B
        MobKind::Squid => &squid::KIND,
        MobKind::GlowSquid => &squid::GLOW,
        MobKind::Cod => &fish::COD,
        MobKind::Salmon => &fish::SALMON,
        MobKind::TropicalFish => &fish::TROPICAL_FISH,
        MobKind::Pufferfish => &fish::PUFFERFISH,
        MobKind::Mooshroom => &mooshroom::KIND,
        MobKind::Ocelot => &ocelot::KIND,
        MobKind::Bat => &bat::KIND,
        MobKind::SnowGolem => &snow_golem::KIND,
        MobKind::Bogged => &bogged::KIND,
        MobKind::Armadillo => &armadillo::KIND,
        MobKind::Camel => &camel::KIND,
        MobKind::Allay => &allay::KIND,

        _ => return None,
    })
}
