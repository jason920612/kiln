//! `WanderingTraderSpawner`: every 1200 ticks the overworld counts down a spawn delay (24000
//! ticks); when it runs out a trader may appear, with a chance that starts at 25% and grows by 25
//! per try up to 75% (back to 25 after a spawn). The trader is spawned 48 blocks around a random
//! player (or around the nearest village bell within 48 blocks of the player), with two trader
//! llamas on leads, a despawn delay of 48000 ticks and a place to walk to.
//!
//! The delay and the chance are `wandering_trader.dat` (`WanderingTraderData`). Vanilla's
//! spawner has an unseeded random; Kiln draws from a random seeded by the world and the time.

use crate::{OVERWORLD_ID, Sim, entities, mobs, poi};
use kiln_entity::leash::{Delayed, LeashData};
use kiln_entity::math::BlockPos;
use kiln_entity::mob::{self, MobKind, kinds::wandering_trader};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_world::{Blocks, ChunkPos};

/// `WanderingTraderSpawner`'s counters.
#[derive(Clone, Debug)]
pub(crate) struct TraderSpawner {
    /// `tickDelay`: ticks until the next count.
    pub tick_delay: i32,
    /// `WanderingTraderData.spawnDelay` and `spawnChance`.
    pub spawn_delay: i32,
    pub spawn_chance: i32,
}

impl Default for TraderSpawner {
    fn default() -> Self {
        TraderSpawner { tick_delay: 1200, spawn_delay: 24000, spawn_chance: 25 }
    }
}

impl TraderSpawner {
    /// `WanderingTraderData.CODEC`.
    pub(crate) fn to_nbt(&self) -> Tag {
        Tag::Compound(vec![("spawn_delay".into(), Tag::Int(self.spawn_delay)), ("spawn_chance".into(), Tag::Int(self.spawn_chance))])
    }

    pub(crate) fn from_nbt(t: &Tag) -> TraderSpawner {
        TraderSpawner {
            tick_delay: 1200,
            spawn_delay: t.get("spawn_delay").and_then(Tag::as_i64).map_or(24000, |v| v as i32),
            spawn_chance: t.get("spawn_chance").and_then(Tag::as_i64).map_or(25, |v| v as i32),
        }
    }
}

/// The stand-in ids of the trader and its llamas until the simulation hands out real ones.
const TRADER_ID: i32 = -3_000_001;

impl Sim {
    /// `WanderingTraderSpawner.tick` (the overworld's), once per tick.
    pub(crate) fn tick_wandering_trader(&mut self) {
        if !self.rule_bool("minecraft:spawn_wandering_traders") {
            return;
        }
        self.trader.tick_delay -= 1;
        if self.trader.tick_delay > 0 {
            return;
        }
        self.trader.tick_delay = 1200;
        let delay = self.trader.spawn_delay - 1200;
        self.trader.spawn_delay = delay;
        if delay > 0 {
            return;
        }
        self.trader.spawn_delay = 24000;
        let chance = self.trader.spawn_chance;
        self.trader.spawn_chance = (chance + 25).clamp(25, 75);
        let seed = self.config.noise.as_ref().map_or(0, |n| n.seed);
        let mut rng = LegacyRandom::new(seed ^ self.game_time.wrapping_mul(0x7472_6164_6572));
        if rng.next_int_bounded(100) > chance {
            return;
        }
        if self.spawn_trader(&mut rng) {
            self.trader.spawn_chance = 25;
        }
    }

    /// `WanderingTraderSpawner.spawn`: true when it is over (spawned, or nobody to spawn for).
    fn spawn_trader(&mut self, rng: &mut LegacyRandom) -> bool {
        let mut players: Vec<&crate::Player> = self.players.values().filter(|p| p.dim == OVERWORLD_ID && !p.dead && !p.disconnected).collect();
        players.sort_unstable_by_key(|p| p.conn);
        if players.is_empty() {
            return true;
        }
        let p = players[rng.next_int_bounded(players.len() as i32) as usize];
        if rng.next_int_bounded(10) != 0 {
            return false;
        }
        let at = [p.pos[0].floor() as i32, p.pos[1].floor() as i32, p.pos[2].floor() as i32];
        // The nearest bell (meeting point) within 48 blocks, else the player's block.
        let kinds = poi::kinds_of(&["minecraft:meeting"]);
        let bells = poi::in_range(&self.dims[OVERWORLD_ID].regions, &kinds, at, 48, kiln_world::poi::Occupancy::Any);
        let dist2 = |r: &[i32; 3]| {
            let (dx, dy, dz) = ((r[0] - at[0]) as i64, (r[1] - at[1]) as i64, (r[2] - at[2]) as i64);
            dx * dx + dy * dy + dz * dz
        };
        let target = bells.iter().map(|r| r.pos).min_by_key(dist2).unwrap_or(at);
        let Some(pos) = self.find_trader_spawn_position(target, 48, rng) else { return false };
        if !self.has_enough_space(pos) {
            return false;
        }
        if self.is_without_trader_spawns(pos) {
            return false;
        }
        let ctx = mobs::difficulty_instance(self.commands.difficulty as u8, self.game_time, 0, 1.0);
        let mut trader = self.new_trader_mob(MobKind::WanderingTrader, TRADER_ID, pos, rng, &ctx);
        {
            let mut kind = std::mem::replace(&mut trader.kind, kiln_entity::EntityKind::MobTicking { gravity: 0.08 });
            if let kiln_entity::EntityKind::Mob(m) = &mut kind {
                wandering_trader::set_despawn_delay(m, 48000);
                wandering_trader::set_wander_target(m, Some(BlockPos::new(target[0], target[1], target[2])));
                wandering_trader::set_home_to(m, BlockPos::new(target[0], target[1], target[2]), 16);
            }
            trader.kind = kind;
        }
        let trader_block = trader.block_position();
        // `tryToSpawnLlamaFor` twice: a trader llama within 4 blocks, on a lead.
        let mut llamas = Vec::new();
        for n in 0..2 {
            let Some(lpos) = self.find_trader_spawn_position([trader_block.x, trader_block.y, trader_block.z], 4, rng) else { continue };
            let mut llama = self.new_trader_mob(MobKind::TraderLlama, TRADER_ID - 1 - n, lpos, rng, &ctx);
            llama.leash = Some(Box::new(LeashData { holder: Some(TRADER_ID), holder_key: Some(Delayed::Holder(0)), delayed: None, angular_momentum: 0.0 }));
            llamas.push(llama);
        }
        let d = &mut self.dims[OVERWORLD_ID];
        let spawn_of = |e: kiln_entity::Entity| {
            let kind = kiln_data::entities::by_name(e.type_name).expect("mob type");
            let p = e.position();
            entities::Spawn { kind, pos: [p.x, p.y, p.z], vel: [0.0; 3], body: entities::Body::Ready(Box::new(e)) }
        };
        d.spawns.push(spawn_of(trader));
        for l in llamas {
            d.spawns.push(spawn_of(l));
        }
        self.materialize_spawns();
        true
    }

    /// A mob of `kind` for the spawner: placed at the middle of the block, facing a random way,
    /// `finalizeSpawn`ed.
    fn new_trader_mob(&self, kind: MobKind, id: i32, pos: [i32; 3], rng: &mut LegacyRandom, ctx: &mob::SpawnContext) -> kiln_entity::Entity {
        let mut e = mob::new(kind, id, 0, rng.next_long());
        e.set_pos(kiln_entity::math::Vec3::new(pos[0] as f64 + 0.5, pos[1] as f64, pos[2] as f64 + 0.5));
        let yaw = mob::mth::wrap_degrees(rng.next_float() * 360.0);
        e.y_rot = yaw;
        e.set_old_pos_and_rot();
        if let Some(m) = mob::data_mut(&mut e) {
            m.y_head_rot = yaw;
            m.y_body_rot = yaw;
            m.y_head_rot_o = yaw;
            m.y_body_rot_o = yaw;
        }
        let mut group = mob::GroupData::default();
        mob::finalize_spawn(&mut e, rng, ctx, &mut group, false);
        e
    }

    /// `findSpawnPositionNear`: ten tries around `near` (within `radius`) for a spot standing
    /// on the ground (`SpawnPlacementTypes.ON_GROUND`) at the motion-blocking height.
    fn find_trader_spawn_position(&self, near: [i32; 3], radius: i32, rng: &mut LegacyRandom) -> Option<[i32; 3]> {
        let d = &self.dims[OVERWORLD_ID];
        let block = |p: [i32; 3]| d.regions.get_block(p[0], p[1], p[2]).unwrap_or(0);
        for _ in 0..10 {
            let x = near[0] + rng.next_int_bounded(radius * 2) - radius;
            let z = near[2] + rng.next_int_bounded(radius * 2) - radius;
            let Some(chunk) = d.regions.chunk(ChunkPos::of_block(x, z)) else { continue };
            let y = chunk.column_height((x & 15) as usize, (z & 15) as usize, |s| {
                kiln_data::block_props::motion_blocking(s) && !kiln_data::blocks_types::block_of(s).name.ends_with("_leaves")
            });
            let pos = [x, y, z];
            if mob::path::valid_spawn(block([x, y - 1, z]), false)
                && mob::path::valid_empty_spawn(block(pos), false)
                && mob::path::valid_empty_spawn(block([x, y + 1, z]), false)
            {
                return Some(pos);
            }
        }
        None
    }

    /// `hasEnoughSpace`: nothing with a collision shape in the 2 by 3 by 2 blocks from `pos`.
    fn has_enough_space(&self, pos: [i32; 3]) -> bool {
        let d = &self.dims[OVERWORLD_ID];
        for dx in 0..=1 {
            for dy in 0..=2 {
                for dz in 0..=1 {
                    let s = d.regions.get_block(pos[0] + dx, pos[1] + dy, pos[2] + dz).unwrap_or(0);
                    if !kiln_data::block_props::collision(s).is_empty() {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// `BiomeTags.WITHOUT_WANDERING_TRADER_SPAWNS` (the biome of the position).
    fn is_without_trader_spawns(&self, pos: [i32; 3]) -> bool {
        let Some(name) = self.biome_name(OVERWORLD_ID, pos) else { return false };
        let Some(id) = kiln_data::synced_id("minecraft:worldgen/biome", name) else { return false };
        kiln_data::registries::TAGS
            .iter()
            .find(|(r, _)| *r == "minecraft:worldgen/biome")
            .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == "minecraft:without_wandering_trader_spawns"))
            .is_some_and(|(_, ids)| ids.contains(&id))
    }

    /// `wandering_trader.dat` of the overworld.
    pub(crate) fn load_trader(&mut self) {
        let Some(dir) = self.storage.as_ref().map(|s| s.dir.clone()) else { return };
        if let Some(data) = kiln_storage::saved_data::read(&dir.join(crate::dimension_dir(crate::DIMENSIONS[OVERWORLD_ID].0)), "wandering_trader") {
            self.trader = TraderSpawner::from_nbt(&data);
        }
    }

    pub(crate) fn save_trader(&mut self) {
        let Some(dir) = self.storage.as_ref().map(|s| s.dir.clone()) else { return };
        if let Err(e) = kiln_storage::saved_data::write(&dir.join(crate::dimension_dir(crate::DIMENSIONS[OVERWORLD_ID].0)), "wandering_trader", self.trader.to_nbt()) {
            tracing::warn!("failed to save the wandering trader data: {e}");
        }
    }
}
