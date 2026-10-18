//! `Mob.getMaxSpawnClusterSize` (wp77): how many of a pack natural spawning places before it stops. Four for a mob; the overrides of 26.3 (read off the
//! vanilla classes): horses six, fish eight (salmon five by `getMaxSchoolSize`), wolves eight, ghasts, happy ghasts and pillagers one.
//! Kiln gave wolves four, so a pack of wolves was half of what vanilla spawns.

use kiln_entity::mob::MobKind;

fn cluster(kind: MobKind) -> i32 {
    kind.ext().map_or(4, |k| k.max_spawn_cluster())
}

#[test]
fn the_pack_sizes_are_vanillas() {
    for (kind, want) in [
        (MobKind::Zombie, 4),
        (MobKind::Pig, 4),
        (MobKind::Wolf, 8),
        (MobKind::Horse, 6),
        (MobKind::Donkey, 6),
        (MobKind::Cod, 8),
        (MobKind::Salmon, 5),
        (MobKind::TropicalFish, 8),
        (MobKind::Pufferfish, 8),
        (MobKind::Ghast, 1),
        (MobKind::HappyGhast, 1),
        (MobKind::Pillager, 1),
    ] {
        assert_eq!(cluster(kind), want, "{kind:?}");
    }
}
