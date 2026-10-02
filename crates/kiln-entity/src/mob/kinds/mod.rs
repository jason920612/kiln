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
pub mod skeleton_horse;
pub mod mule;
pub mod strider;
pub mod iron_golem;
pub mod villager;
pub mod piglin;
pub mod hoglin;
pub mod silverfish;
// -- slice 3: raids
pub mod raider;
pub mod pillager;
pub mod vindicator;
pub mod evoker;
pub mod vex;
pub mod ravager;
pub mod illusioner;

// -- slice 3: the end
pub mod ender_dragon;

// -- slice 3: wither and guardians
pub mod wither;
pub mod guardian;

// -- slice 3: warden
pub mod warden;

// -- slice 3: common mobs A
pub mod common_a;
pub mod rabbit;
pub mod polar_bear;
pub mod turtle;
pub mod fox;
pub mod panda;

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
pub mod breeze;
pub mod creaking;
pub mod creaking_heart;
pub mod sniffer;

// -- wp28: brain mobs
pub mod piglin_brute;
pub mod zoglin;
pub mod axolotl;
pub mod goat;
pub mod frog;
pub mod tadpole;

// -- wp30: llamas
pub mod llama;

// -- wp32: wandering traders
pub mod wandering_trader;
// -- wp32: parrots
pub mod parrot;


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
        MobKind::SkeletonHorse => &skeleton_horse::KIND,
        MobKind::Strider => &strider::KIND,
        MobKind::IronGolem => &iron_golem::KIND,
        MobKind::Villager => &villager::KIND,
        MobKind::Piglin => &piglin::KIND,
        MobKind::Hoglin => &hoglin::KIND,
        MobKind::Silverfish => &silverfish::KIND,
        // -- slice 3: raids
        MobKind::Pillager => &pillager::KIND,
        MobKind::Vindicator => &vindicator::KIND,
        MobKind::Evoker => &evoker::KIND,
        MobKind::Vex => &vex::KIND,
        MobKind::Ravager => &ravager::KIND,
        MobKind::Illusioner => &illusioner::KIND,

        // -- slice 3: the end
        MobKind::EnderDragon => &ender_dragon::KIND,

        // -- slice 3: wither and guardians
        MobKind::Wither => &wither::KIND,
        MobKind::Guardian => &guardian::GUARDIAN,
        MobKind::ElderGuardian => &guardian::ELDER,

        // -- slice 3: warden
        MobKind::Warden => &warden::KIND,

        // -- slice 3: common mobs A
        MobKind::Rabbit => &rabbit::KIND,
        MobKind::PolarBear => &polar_bear::KIND,
        MobKind::Turtle => &turtle::KIND,
        MobKind::Fox => &fox::KIND,
        MobKind::Panda => &panda::KIND,

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
        MobKind::Breeze => &breeze::KIND,
        MobKind::Creaking => &creaking::KIND,
        MobKind::Sniffer => &sniffer::KIND,

        // -- wp28: brain mobs
        MobKind::PiglinBrute => &piglin_brute::KIND,
        MobKind::Zoglin => &zoglin::KIND,
        MobKind::Axolotl => &axolotl::KIND,
        MobKind::Goat => &goat::KIND,
        MobKind::Frog => &frog::KIND,
        MobKind::Tadpole => &tadpole::KIND,

        // -- wp30: llamas
        MobKind::Llama => &llama::LLAMA,
        MobKind::TraderLlama => &llama::TRADER_LLAMA,

        // -- wp32: wandering traders
        MobKind::WanderingTrader => &wandering_trader::KIND,
        // -- wp32: parrots
        MobKind::Parrot => &parrot::KIND,

        _ => return None,
    })
}
