//! The overworld's village spawners: `CatSpawner` (every minute a stray cat may appear near a village, or
//! in a swamp hut) and `VillageSiege` (on one night in ten, zombies come out of the dark to a village where
//! a player is, twenty of them, one every third tick).
//!
//! Vanilla's spawners draw from the level's one random; these draw from a random seeded by the world and
//! the time, like the other spawners here (`trader.rs`, the patrols).

use crate::{OVERWORLD_ID, Sim, mobs, poi, spawner};
use kiln_blocks::BlockPos;
use kiln_entity::mob::{MobKind, mth};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_world::{Blocks as _, ChunkPos};

/// `VillageSiege.State`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SiegeState {
    Tonight,
    Done,
}

/// `VillageSiege`'s fields.
#[derive(Clone, Debug)]
pub(crate) struct Siege {
    state: SiegeState,
    has_setup: bool,
    next_spawn_time: i32,
    zombies_to_spawn: i32,
    spawn: [i32; 3],
}

impl Default for Siege {
    fn default() -> Self {
        Siege { state: SiegeState::Done, has_setup: false, next_spawn_time: 0, zombies_to_spawn: 0, spawn: [0; 3] }
    }
}

impl Siege {
    /// For tests: the state as (tonight, zombies left, spawn point).
    pub(crate) fn describe(&self) -> (bool, i32, [i32; 3]) {
        (self.state == SiegeState::Tonight, self.zombies_to_spawn, self.spawn)
    }
}

/// The stand-in id of a spawner's new mob until the simulation hands out real ones.
const SPAWNER_ID: i32 = -3_100_001;

impl Sim {
    /// `ServerLevel.isCloseToVillage(pos, sections)`.
    fn close_to_village(&self, pos: [i32; 3], sections: i32) -> bool {
        sections <= 6 && poi::sections_to_village(&self.dims[OVERWORLD_ID].regions, pos) <= sections
    }

    /// Whether the chunks around `pos` (`radius` blocks) are loaded (`hasChunksAt`).
    fn chunks_loaded_around(&self, pos: [i32; 3], radius: i32) -> bool {
        let d = &self.dims[OVERWORLD_ID];
        for cx in (pos[0] - radius) >> 4..=(pos[0] + radius) >> 4 {
            for cz in (pos[2] - radius) >> 4..=(pos[2] + radius) >> 4 {
                if d.regions.chunk(ChunkPos::new(cx, cz)).is_none() {
                    return false;
                }
            }
        }
        true
    }

    /// The living cats whose box meets `pos` inflated by (`h`, `v`, `h`).
    fn cats_near(&self, pos: [i32; 3], h: f64, v: f64) -> usize {
        let (lo, hi) = ([pos[0] as f64 - h, pos[1] as f64 - v, pos[2] as f64 - h], [pos[0] as f64 + 1.0 + h, pos[1] as f64 + 1.0 + v, pos[2] as f64 + 1.0 + h]);
        let mut n = 0;
        for region in self.dims[OVERWORLD_ID].regions.iter() {
            for e in region.part().0.list.iter().filter(|e| !e.removed) {
                let Some(phys) = e.phys.as_deref() else { continue };
                if phys.type_name != "minecraft:cat" {
                    continue;
                }
                let b = phys.bounding_box();
                if b.max_x > lo[0] && b.min_x < hi[0] && b.max_y > lo[1] && b.min_y < hi[1] && b.max_z > lo[2] && b.min_z < hi[2] {
                    n += 1;
                }
            }
        }
        n
    }

    /// `CatSpawner.tick` (the overworld's), once per tick.
    pub(crate) fn tick_cat_spawner(&mut self) {
        self.cat_next_tick -= 1;
        if self.cat_next_tick > 0 {
            return;
        }
        self.cat_next_tick = 1200;
        // `ServerLevel.getRandomPlayer`: an alive player of the level.
        let mut players: Vec<(crate::ConnId, [f64; 3])> = self.players.values().filter(|p| p.dim == OVERWORLD_ID && !p.dead && !p.disconnected).map(|p| (p.conn, p.pos)).collect();
        players.sort_unstable_by_key(|p| p.0);
        if players.is_empty() {
            return;
        }
        let seed = self.config.noise.as_ref().map_or(0, |n| n.seed);
        let mut rng = LegacyRandom::new(seed ^ self.game_time.wrapping_mul(0x0063_6174_7370));
        let ppos = players[rng.next_int_bounded(players.len() as i32) as usize].1;
        let dx = (8 + rng.next_int_bounded(24)) * if rng.next_bool() { -1 } else { 1 };
        let dz = (8 + rng.next_int_bounded(24)) * if rng.next_bool() { -1 } else { 1 };
        let pos = [ppos[0].floor() as i32 + dx, ppos[1].floor() as i32, ppos[2].floor() as i32 + dz];
        if !self.chunks_loaded_around(pos, 10) {
            return;
        }
        // `SpawnPlacements.isSpawnPositionOk(CAT, level, pos)`.
        let ok = self.with_level_in(OVERWORLD_ID, pos, |l| spawner::spawn_position_ok(l, BlockPos::new(pos[0], pos[1], pos[2]), MobKind::Cat)).unwrap_or(false);
        if !ok {
            return;
        }
        if self.close_to_village(pos, 2) {
            // `spawnInVillage`: more than four occupied beds within 48 blocks, and fewer than five cats around.
            let beds = poi::in_range(&self.dims[OVERWORLD_ID].regions, &poi::kinds_of(&["minecraft:home"]), pos, 48, kiln_world::poi::Occupancy::IsOccupied).len();
            if beds > 4 && self.cats_near(pos, 48.0, 8.0) < 5 {
                self.spawn_cat(pos, false, &mut rng);
            }
        } else if self.in_cats_spawn_in_structure(pos) {
            // `spawnInHut`: no cat within 16 blocks yet.
            if self.cats_near(pos, 16.0, 8.0) == 0 {
                self.spawn_cat(pos, true, &mut rng);
            }
        }
    }

    /// `getStructureWithPieceAt(pos, #minecraft:cats_spawn_in)`.
    fn in_cats_spawn_in_structure(&self, pos: [i32; 3]) -> bool {
        let Some(ids) = crate::world_state::worldgen_tag("worldgen/structure", "minecraft:cats_spawn_in") else { return false };
        let d = &self.dims[OVERWORLD_ID];
        let structures = |c: ChunkPos| d.regions.chunk(c).and_then(|ch| ch.structures.as_deref());
        crate::structure_spawns::piece_at(&structures, pos, &ids)
    }

    /// `CatSpawner.spawnCat`: a cat made like a natural spawn, a persistent one in a hut.
    fn spawn_cat(&mut self, pos: [i32; 3], persistent: bool, rng: &mut LegacyRandom) {
        let inhabited = 0;
        let mut ctx = mobs::difficulty_instance(self.commands.difficulty as u8, self.game_time, inhabited, spawner::moon_brightness(self.day_time));
        ctx.biome = self.biome_name(OVERWORLD_ID, pos).and_then(|n| kiln_data::synced_id("minecraft:worldgen/biome", n));
        let black_cat = self.with_level_in(OVERWORLD_ID, pos, |l| spawner::black_cat_structure(l, pos)).unwrap_or(false);
        let fin = mobs::Finalize { ctx, seed: rng.next_long(), persistent, natural: true, monsters_disabled: false, camel_space: false, black_cat };
        let d = &mut self.dims[OVERWORLD_ID];
        d.spawns.push(mobs::spawn(MobKind::Cat, [pos[0] as f64 + 0.5, pos[1] as f64, pos[2] as f64 + 0.5], Some(0.0), Some(fin)));
        self.materialize_spawns();
    }

    /// `VillageSiege.tick` (the overworld's), once per tick.
    pub(crate) fn tick_village_siege(&mut self) {
        let spawn_enemies = self.commands.difficulty as u8 != 0 && self.rule_bool("minecraft:spawn_monsters");
        let sky_darken = crate::weather::sky_darken(OVERWORLD_ID, self.day_time, &self.level_weather[OVERWORLD_ID]);
        // `isBrightOutside`.
        if sky_darken < 4 || !spawn_enemies {
            self.siege.state = SiegeState::Done;
            self.siege.has_setup = false;
            return;
        }
        let seed = self.config.noise.as_ref().map_or(0, |n| n.seed);
        let mut rng = LegacyRandom::new(seed ^ self.game_time.wrapping_mul(0x7369_6567_65));
        // `isAtTimeMarker(ROLL_VILLAGE_SIEGE)`: the tick the day reaches it.
        if self.day_time.rem_euclid(24000) == 18000 {
            self.siege.state = if rng.next_int_bounded(10) == 0 { SiegeState::Tonight } else { SiegeState::Done };
        }
        if self.siege.state == SiegeState::Done {
            return;
        }
        if !self.siege.has_setup {
            if self.try_to_setup_siege(&mut rng) {
                self.siege.has_setup = true;
            } else {
                return;
            }
        }
        if self.siege.next_spawn_time > 0 {
            self.siege.next_spawn_time -= 1;
            return;
        }
        self.siege.next_spawn_time = 2;
        if self.siege.zombies_to_spawn > 0 {
            self.siege_try_spawn(&mut rng);
            self.siege.zombies_to_spawn -= 1;
        } else {
            self.siege.state = SiegeState::Done;
        }
    }

    /// `VillageSiege.tryToSetupSiege`: the first player (not a spectator) standing in a village outside the
    /// biomes without sieges gets a spawn point 32 blocks away.
    fn try_to_setup_siege(&mut self, rng: &mut LegacyRandom) -> bool {
        let mut players: Vec<(crate::ConnId, [f64; 3], u8)> = self.players.values().filter(|p| p.dim == OVERWORLD_ID).map(|p| (p.conn, p.pos, p.game_mode)).collect();
        players.sort_unstable_by_key(|p| p.0);
        for (_, ppos, mode) in players {
            if mode == 3 {
                continue;
            }
            let p = [ppos[0].floor() as i32, ppos[1].floor() as i32, ppos[2].floor() as i32];
            if !self.close_to_village(p, 1) {
                continue;
            }
            if self.biome_in_tag(p, "minecraft:without_zombie_sieges") {
                continue;
            }
            for _ in 0..10 {
                let angle = rng.next_float() * 6.2831855;
                self.siege.spawn = [p[0] + (mth::cos(angle as f64) * 32.0f32).floor() as i32, p[1], p[2] + (mth::sin(angle as f64) * 32.0f32).floor() as i32];
                if self.siege_spawn_pos(self.siege.spawn, rng).is_some() {
                    self.siege.next_spawn_time = 0;
                    self.siege.zombies_to_spawn = 20;
                    break;
                }
            }
            return true;
        }
        false
    }

    /// `VillageSiege.trySpawn`: a zombie at a spot near the siege's spawn point, facing a random way.
    fn siege_try_spawn(&mut self, rng: &mut LegacyRandom) {
        let Some(pos) = self.siege_spawn_pos(self.siege.spawn, rng) else { return };
        let ctx = mobs::difficulty_instance(self.commands.difficulty as u8, self.game_time, 0, 1.0);
        let fin = mobs::Finalize { ctx, seed: rng.next_long(), persistent: false, natural: false, monsters_disabled: false, camel_space: false, black_cat: false };
        let yaw = rng.next_float() * 360.0;
        self.dims[OVERWORLD_ID].spawns.push(mobs::spawn(MobKind::Zombie, pos, Some(yaw), Some(fin)));
        self.materialize_spawns();
    }

    /// `VillageSiege.findRandomSpawnPos`: ten tries within 8 blocks of `center` for a spot of a village on the
    /// surface where a zombie may spawn (`Monster.checkMonsterSpawnRules`, dark enough).
    fn siege_spawn_pos(&mut self, center: [i32; 3], rng: &mut LegacyRandom) -> Option<[f64; 3]> {
        for _ in 0..10 {
            let x = center[0] + rng.next_int_bounded(16) - 8;
            let z = center[2] + rng.next_int_bounded(16) - 8;
            let y = {
                let d = &self.dims[OVERWORLD_ID];
                let Some(chunk) = d.regions.chunk(ChunkPos::of_block(x, z)) else { continue };
                // `Heightmap.Types.WORLD_SURFACE`.
                chunk.column_height((x & 15) as usize, (z & 15) as usize, |s| !kiln_data::blocks_types::is_air(s))
            };
            let pos = [x, y, z];
            if !self.close_to_village(pos, 1) {
                continue;
            }
            let ok = self.with_level_in(OVERWORLD_ID, pos, |l| spawner::check_spawn_rules(l, BlockPos::new(x, y, z), MobKind::Zombie, rng)).unwrap_or(false);
            if ok {
                return Some([x as f64 + 0.5, y as f64, z as f64 + 0.5]);
            }
        }
        None
    }

    /// Whether the biome at `pos` of the overworld is in the biome tag `tag`.
    fn biome_in_tag(&self, pos: [i32; 3], tag: &str) -> bool {
        let Some(name) = self.biome_name(OVERWORLD_ID, pos) else { return false };
        let Some(id) = kiln_data::synced_id("minecraft:worldgen/biome", name) else { return false };
        kiln_data::registries::TAGS
            .iter()
            .find(|(r, _)| *r == "minecraft:worldgen/biome")
            .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
            .is_some_and(|(_, ids)| ids.contains(&id))
    }
}
