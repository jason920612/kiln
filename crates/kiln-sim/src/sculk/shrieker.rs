//! Sculk shriekers (`SculkShriekerBlockEntity`): a player's vibration (or step) makes them
//! shriek, raising the warning level players nearby share (`WardenSpawnTracker`); when the
//! shriek ends, a shrieker that can summon answers with darkness and, at warning level 4, a
//! warden digging out of the ground (`SpawnUtil.trySpawnMob`, `ON_TOP_OF_COLLIDER`).

use super::{Kind, center, post};
use crate::Player;
use crate::blocks::RegionLevel;
use kiln_blocks::{BlockPos, Level, flags};
use kiln_entity::vibration::Context;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;

/// `WardenSpawnTracker`: a player's warning level toward summoning a warden.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WardenSpawnTracker {
    pub ticks_since_last_warning: i32,
    pub warning_level: i32,
    pub cooldown_ticks: i32,
}

impl WardenSpawnTracker {
    /// `WardenSpawnTracker.CODEC` (`warden_spawn_tracker` in player data).
    pub fn load(t: Option<&Tag>) -> WardenSpawnTracker {
        let int = |k: &str| t.and_then(|t| t.get(k)).and_then(Tag::as_i64).map_or(0, |v| v.max(0) as i32);
        WardenSpawnTracker { ticks_since_last_warning: int("ticks_since_last_warning"), warning_level: int("warning_level"), cooldown_ticks: int("cooldown_ticks") }
    }

    pub fn to_nbt(&self) -> Tag {
        Tag::Compound(vec![
            ("ticks_since_last_warning".into(), Tag::Int(self.ticks_since_last_warning)),
            ("warning_level".into(), Tag::Int(self.warning_level)),
            ("cooldown_ticks".into(), Tag::Int(self.cooldown_ticks)),
        ])
    }

    /// `tick` (every player tick): the level drops one every 12000 ticks without a warning.
    pub fn tick(&mut self) {
        if self.ticks_since_last_warning >= 12000 {
            self.warning_level = (self.warning_level - 1).clamp(0, 4);
            self.ticks_since_last_warning = 0;
        } else {
            self.ticks_since_last_warning += 1;
        }
        if self.cooldown_ticks > 0 {
            self.cooldown_ticks -= 1;
        }
    }

    fn increase(&mut self) {
        if self.cooldown_ticks <= 0 {
            self.ticks_since_last_warning = 0;
            self.cooldown_ticks = 200;
            self.warning_level = (self.warning_level + 1).clamp(0, 4);
        }
    }
}

/// `SculkShriekerBlockEntity.tryShriek` for player `player` (entity id): `wardens` are the
/// positions of the region's wardens.
pub(crate) fn try_shriek(level: &mut RegionLevel, players: &mut [&mut Player], wardens: &[[f64; 3]], pos: BlockPos, player: i32) {
    let s = level.block(pos);
    if level.blocks.sculk.map.get(&pos).is_none_or(|b| b.kind != Kind::Shrieker) || kiln_blocks::state::get_bool(s, "shrieking") {
        return;
    }
    let Some(i) = players.iter().position(|p| p.entity_id == player) else { return };
    if let Some(be) = level.blocks.sculk.map.get_mut(&pos) {
        be.warning_level = 0;
        be.dirty = true;
    }
    if can_respond(level, s) {
        let Some(warning) = try_warn(players, wardens, pos, i) else { return };
        if let Some(be) = level.blocks.sculk.map.get_mut(&pos) {
            be.warning_level = warning;
        }
    }
    shriek(level, pos, s, players[i]);
}

/// `canRespond`: a shrieker that can summon, not in peaceful, with `spawn_wardens` on.
fn can_respond(level: &RegionLevel, s: u16) -> bool {
    kiln_blocks::state::get_bool(s, "can_summon") && level.env.mobs.difficulty != 0 && level.env.mobs.spawn_wardens
}

/// `WardenSpawnTracker.tryWarn`: no warden within a 48-block box, nobody near (or the
/// trigger) on cooldown: the highest warning level near goes up by one and every player near
/// takes it.
fn try_warn(players: &mut [&mut Player], wardens: &[[f64; 3]], pos: BlockPos, trigger: usize) -> Option<i32> {
    let c = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
    if wardens.iter().any(|w| (0..3).all(|i| (w[i] - c[i]).abs() < 24.0)) {
        return None;
    }
    let mut near: Vec<usize> = (0..players.len())
        .filter(|&i| {
            let p = &players[i];
            !p.dead && !p.disconnected && p.game_mode != 3 && (0..3).map(|k| (p.pos[k] - c[k]).powi(2)).sum::<f64>() < 16.0 * 16.0
        })
        .collect();
    if !near.contains(&trigger) {
        near.push(trigger);
    }
    if near.iter().any(|&i| players[i].warden_tracker.cooldown_ticks > 0) {
        return None;
    }
    // `max(comparingInt(getWarningLevel))`: the first of the highest.
    let best = near.iter().copied().reduce(|a, b| if players[b].warden_tracker.warning_level > players[a].warden_tracker.warning_level { b } else { a })?;
    let mut tracker = players[best].warden_tracker;
    tracker.increase();
    for &i in &near {
        players[i].warden_tracker = tracker;
    }
    Some(tracker.warning_level)
}

/// `shriek`: the shrieking state for 90 ticks, level event 3007 and the shriek game event.
fn shriek(level: &mut RegionLevel, pos: BlockPos, s: u16, source: &Player) {
    kiln_blocks::set_block(level, pos, kiln_blocks::state::set_bool(s, "shrieking", true), flags::CLIENTS);
    kiln_blocks::schedule_block_tick(level, pos, kiln_blocks::BlockId::of(s), 90, kiln_blocks::TickPriority::Normal);
    level.effect(kiln_blocks::Effect::LevelEvent { id: 3007, pos, data: 0 });
    let source = crate::blocks::player_source(source);
    post(level, "minecraft:shriek", center(pos), Context { source: Some(source), affected_state: None });
}

/// `SculkShriekerBlockEntity.tryRespond` (the shriek ended, or the shrieking block went
/// away): the region answers with darkness and maybe a warden ([`respond`]).
pub(crate) fn try_respond(level: &mut RegionLevel, pos: BlockPos) {
    let Some(be) = level.blocks.sculk.map.get(&pos).filter(|b| b.kind == Kind::Shrieker) else { return };
    let warning = be.warning_level;
    let s = level.block(pos);
    if can_respond(level, s) && warning > 0 {
        level.out.responds.push((pos, warning));
    }
}

/// The sound a shrieker answers with at each warning level when no warden comes.
fn reply_sound(warning: i32) -> Option<&'static str> {
    Some(match warning {
        1 => "minecraft:entity.warden.nearby_close",
        2 => "minecraft:entity.warden.nearby_closer",
        3 => "minecraft:entity.warden.nearby_closest",
        4 => "minecraft:entity.warden.listening_angry",
        _ => return None,
    })
}

/// What a shrieker's answer does: a warden to summon (where it digs out), or a reply sound;
/// darkness for the survival players within 40 blocks either way.
pub(crate) enum Answer {
    Warden([f64; 3]),
    Sound { at: [f64; 3], sound: &'static str },
}

/// The answer of the shrieker at `pos` with `warning`: at level 4 twenty tries to find ground
/// for a warden within 5 blocks sideways and 6 up or down (`trySummonWarden`), else the reply
/// sound 10 blocks around. Vanilla draws from the level random; Kiln from one seeded by the
/// position and time (an approximation, I class).
pub(crate) fn answer(level: &RegionLevel, pos: BlockPos, warning: i32, occupied: &dyn Fn([f64; 3]) -> bool) -> Option<Answer> {
    let mut rng = crate::container::pos_random(level, pos, 0x5348_5249);
    if warning >= 4 {
        for _ in 0..20 {
            let dx = between(&mut rng, -5, 5);
            let dz = between(&mut rng, -5, 5);
            let Some(y) = spawn_ground(level, BlockPos::new(pos.x + dx, pos.y + 6, pos.z + dz), 6) else { continue };
            let at = [(pos.x + dx) as f64 + 0.5, y as f64, (pos.z + dz) as f64 + 0.5];
            // `checkSpawnObstruction`: no entity and no block in the warden's box.
            if warden_fits(level, BlockPos::new(pos.x + dx, y, pos.z + dz)) && !occupied(at) {
                return Some(Answer::Warden(at));
            }
        }
    }
    let sound = reply_sound(warning)?;
    let x = pos.x + between(&mut rng, -10, 10);
    let y = pos.y + between(&mut rng, -10, 10);
    let z = pos.z + between(&mut rng, -10, 10);
    Some(Answer::Sound { at: [x as f64, y as f64, z as f64], sound })
}

/// `Mth.randomBetweenInclusive`.
fn between(rng: &mut LegacyRandom, lo: i32, hi: i32) -> i32 {
    rng.next_int_bounded(hi - lo + 1) + lo
}

/// `SpawnUtil.moveToPossibleSpawnPosition` with `ON_TOP_OF_COLLIDER`: from `start` down, the
/// first block with a full top face under a block without collision; the y above it.
fn spawn_ground(level: &RegionLevel, start: BlockPos, range: i32) -> Option<i32> {
    let mut above = level.block(start);
    let mut p = start;
    for _ in (-range..=range).rev() {
        p = p.below();
        let s = level.block(p);
        if kiln_data::block_props::collision(above).is_empty() && full_top(s) {
            return Some(p.y + 1);
        }
        above = s;
    }
    None
}

/// `Block.isFaceFull(collisionShape, UP)` (Kiln reads the full sturdy face).
fn full_top(s: u16) -> bool {
    kiln_blocks::behaviour::sturdy(s, kiln_blocks::Direction::Up, kiln_data::block_logic::Support::Full)
}

/// The warden's box (0.9 x 2.9) at `feet` touches no block collision.
fn warden_fits(level: &RegionLevel, feet: BlockPos) -> bool {
    let (min, max) = ([feet.x as f64 + 0.05, feet.y as f64, feet.z as f64 + 0.05], [feet.x as f64 + 0.95, feet.y as f64 + 2.9, feet.z as f64 + 0.95]);
    for x in min[0].floor() as i32..=max[0].floor() as i32 {
        for y in min[1].floor() as i32 - 1..=max[1].floor() as i32 {
            for z in min[2].floor() as i32..=max[2].floor() as i32 {
                let s = level.block(BlockPos::new(x, y, z));
                for b in kiln_data::block_props::collision(s) {
                    let lo = [x as f64 + b[0] as f64, y as f64 + b[1] as f64, z as f64 + b[2] as f64];
                    let hi = [x as f64 + b[3] as f64, y as f64 + b[4] as f64, z as f64 + b[5] as f64];
                    if (0..3).all(|i| lo[i] < max[i] && hi[i] > min[i]) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// `Warden.applyDarknessAround` (`MobEffectUtil.addEffectToPlayersAround`): darkness for 260
/// ticks, hidden, for survival and adventure players within `radius` of `at` that do not have
/// it or whose darkness ends within 199 ticks.
pub(crate) fn darkness_around(players: &mut [&mut Player], at: [f64; 3], radius: f64) {
    let Some(id) = crate::effects::effect_id("minecraft:darkness") else { return };
    for p in players.iter_mut() {
        let near = (0..3).map(|i| (p.pos[i] - at[i]).powi(2)).sum::<f64>() < radius * radius;
        if p.dead || p.disconnected || !matches!(p.game_mode, 0 | 2) || !near {
            continue;
        }
        let refresh = match p.effects.get(&id) {
            None => true,
            Some(e) => e.amplifier < 0 || (e.duration != crate::effects::INFINITE && e.duration <= 199),
        };
        if refresh {
            p.add_effect(crate::effects::Effect::new(id, 260, 0, false, false, false));
        }
    }
}

/// `trySummonWarden` found ground: a warden digs out there (`finalizeSpawn` with the
/// `TRIGGERED` reason: emerging, with the agitated sound).
pub(crate) fn summon_warden(level: &mut RegionLevel, at: [f64; 3], spawns: &mut Vec<crate::entities::Spawn>) {
    let feet = BlockPos::new(at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32);
    let seed = crate::container::pos_random(level, feet, 0x7761_7264).next_long();
    let mut e = kiln_entity::mob::new(kiln_entity::mob::MobKind::Warden, 0, 0, seed);
    e.set_pos(kiln_entity::math::Vec3::new(at[0], at[1], at[2]));
    e.set_old_pos_and_rot();
    let ctx = crate::mobs::difficulty_instance(level.env.mobs.difficulty, level.env.game_time, 0, 1.0);
    let mut r = LegacyRandom::new(seed ^ 0x5eed);
    kiln_entity::mob::finalize_spawn(&mut e, &mut r, &ctx, &mut kiln_entity::mob::GroupData::default(), false);
    kiln_entity::mob::kinds::warden::emerge(&mut e);
    reply(level, at, "minecraft:entity.warden.agitated");
    spawns.push(crate::entities::Spawn {
        kind: &kiln_data::entities::types::WARDEN,
        pos: at,
        vel: [0.0; 3],
        body: crate::entities::Body::Ready(Box::new(e)),
    });
}

/// A sound for the players in range (`playSound(null, x, y, z, sound, HOSTILE, 5, 1)`).
pub(crate) fn reply(level: &mut RegionLevel, at: [f64; 3], sound: &'static str) {
    let Some(id) = kiln_data::builtin_id("minecraft:sound_event", sound) else { return };
    let seed = crate::container::pos_random(level, BlockPos::new(at[0] as i32, at[1] as i32, at[2] as i32), 0x736f).next_long();
    let pkt = kiln_proto::packets::world_fx::sound(&kiln_proto::packets::world_fx::Sound::Registered(id), kiln_proto::packets::world_fx::SoundSource::Hostile, at, 5.0, 1.0, seed);
    level.out.packets.push((at, 16.0 * 5.0, pkt));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trackers_decay_and_cool_down() {
        let mut t = WardenSpawnTracker { ticks_since_last_warning: 11999, warning_level: 3, cooldown_ticks: 2 };
        t.tick();
        assert_eq!((t.ticks_since_last_warning, t.warning_level, t.cooldown_ticks), (12000, 3, 1));
        t.tick();
        assert_eq!((t.ticks_since_last_warning, t.warning_level, t.cooldown_ticks), (0, 2, 0));
        t.increase();
        assert_eq!((t.warning_level, t.cooldown_ticks), (3, 200));
        t.increase();
        assert_eq!(t.warning_level, 3, "on cooldown");
        assert_eq!(WardenSpawnTracker::load(Some(&t.to_nbt())), t);
    }
}
