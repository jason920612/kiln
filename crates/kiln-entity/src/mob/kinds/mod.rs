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

// -- slice 3: common mobs B
pub mod squid;


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

        // -- slice 3: common mobs B
        MobKind::Squid => &squid::KIND,
        MobKind::GlowSquid => &squid::GLOW,

        _ => return None,
    })
}
