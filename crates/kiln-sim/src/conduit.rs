//! Conduits (`ConduitBlockEntity.serverTick`). Every 40 game ticks a conduit looks at its frame: the 3x3x3 around it must be water, and the
//! prismarine blocks (`#conduit_effect_block`) on the edges of the 5x5x5 around that count: 16 of them activate it (a sound), 42 let it
//! hunt. An active conduit gives Conduit Power (260 ticks, ambient) to the players in water or rain within `count / 7 * 16` blocks; a
//! hunting one picks (or keeps) a hostile mob in water within 8 blocks and hurts it by 4 (magic) every 40 ticks. It hums every 80
//! ticks and once more at random.
//!
//! The block-entity phase has the entities and the players, so all of it happens here; the target is told to the clients in the block
//! entity's update tag (`Target`).

use crate::Player;
use crate::blocks::RegionLevel;
use crate::container::BeKind;
use crate::entities::{Entities, Spawn};
use kiln_blocks::{BlockPos, Effect, Level};
use kiln_proto::nbt::Tag;
use kiln_world::ChunkPos;
use uuid::Uuid;

/// `ConduitBlockEntity`'s fields.
#[derive(Debug, Clone, Default)]
pub(crate) struct State {
    pub tick_count: i32,
    pub active: bool,
    pub hunting: bool,
    /// `destroyTarget`.
    pub target: Option<Uuid>,
    /// `nextAmbientSoundActivation`.
    pub next_ambient: i64,
}

impl State {
    /// `loadAdditional`: `Target` is a UUID.
    pub fn load(nbt: &Tag) -> State {
        State { target: nbt.get("Target").and_then(uuid_of), ..State::default() }
    }

    pub fn save(&self, out: &mut Vec<(String, Tag)>) {
        if let Some(t) = &self.target {
            out.push(("Target".into(), uuid_tag(t)));
        }
    }
}

fn uuid_tag(u: &Uuid) -> Tag {
    let (hi, lo) = u.as_u64_pair();
    Tag::IntArray(vec![(hi >> 32) as i32, hi as i32, (lo >> 32) as i32, lo as i32])
}

fn uuid_of(t: &Tag) -> Option<Uuid> {
    match t {
        Tag::IntArray(v) if v.len() == 4 => {
            let hi = ((v[0] as u32 as u64) << 32) | v[1] as u32 as u64;
            let lo = ((v[2] as u32 as u64) << 32) | v[3] as u32 as u64;
            Some(Uuid::from_u64_pair(hi, lo))
        }
        _ => None,
    }
}

/// `Level.isWaterAt`.
fn is_water(level: &RegionLevel, pos: BlockPos) -> bool {
    kiln_data::block_logic::fluid(level.block(pos)).kind == kiln_data::block_logic::FluidKind::Water
}

/// `updateShape`: the number of frame blocks (`effectBlocks.size()`); none while any of the 3x3x3 is not water.
fn frame_blocks(level: &RegionLevel, pos: BlockPos) -> usize {
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                if !is_water(level, BlockPos::new(pos.x + x, pos.y + y, pos.z + z)) {
                    return 0;
                }
            }
        }
    }
    let mut n = 0;
    for x in -2i32..=2 {
        for y in -2i32..=2 {
            for z in -2i32..=2 {
                let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
                if ax <= 1 && ay <= 1 && az <= 1 {
                    continue;
                }
                let on_edge = (x == 0 && (ay == 2 || az == 2)) || (y == 0 && (ax == 2 || az == 2)) || (z == 0 && (ax == 2 || ay == 2));
                if on_edge && kiln_blocks::tags::is(level.block(BlockPos::new(pos.x + x, pos.y + y, pos.z + z)), "minecraft:conduit_effect_block") {
                    n += 1;
                }
            }
        }
    }
    n
}

/// `Entity.isInWaterOrRain` of a mob.
fn mob_in_water_or_rain(level: &RegionLevel, e: &crate::entities::Entity) -> bool {
    let Some(p) = e.phys.as_deref() else { return false };
    if p.is_in_water() {
        return true;
    }
    let bb = p.bounding_box();
    crate::weather::in_rain(level.cells, level.env, e.pos, bb.max_y)
}

/// The block entity phase of every conduit in a ticking chunk.
pub(crate) fn tick(
    level: &mut RegionLevel,
    entities: &mut Entities,
    players: &mut [&mut Player],
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<crate::health::Death>,
    ticking: &crate::blocks::Ticking,
) {
    let conduits: Vec<BlockPos> = level
        .blocks
        .containers
        .map
        .iter()
        .filter(|(p, c)| c.kind == BeKind::Conduit && ticking.contains(ChunkPos::of_block(p.x, p.z)))
        .map(|(p, _)| *p)
        .collect();
    for pos in conduits {
        serve(level, entities, players, spawns, deaths, pos);
    }
}

/// `ConduitBlockEntity.serverTick`.
fn serve(level: &mut RegionLevel, entities: &mut Entities, players: &mut [&mut Player], spawns: &mut Vec<Spawn>, deaths: &mut Vec<crate::health::Death>, pos: BlockPos) {
    let now = level.env.game_time;
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    let Some(st) = c.conduit.as_deref_mut() else { return };
    st.tick_count += 1;
    if now % 40 == 0 {
        let size = frame_blocks(level, pos);
        let active = size >= 16;
        let st = level.blocks.containers.get_mut(pos).and_then(|c| c.conduit.as_deref_mut()).expect("conduit");
        let changed = active != st.active;
        st.active = active;
        st.hunting = size >= 42;
        if changed {
            level.effect(Effect::Sound { pos, sound: if active { "minecraft:block.conduit.activate" } else { "minecraft:block.conduit.deactivate" }, volume: 1.0, pitch: 1.0 });
        }
        if active {
            apply_effects(level, players, pos, size);
            update_and_attack_target(level, entities, players, spawns, deaths, pos, size >= 42);
        }
    }
    let Some(st) = level.blocks.containers.get_mut(pos).and_then(|c| c.conduit.as_deref_mut()) else { return };
    if !st.active {
        return;
    }
    let mut ambient_short = false;
    if now > st.next_ambient {
        let mut rng = crate::container::pos_random(level, pos, 0x636f6e64);
        let st = level.blocks.containers.get_mut(pos).and_then(|c| c.conduit.as_deref_mut()).expect("conduit");
        st.next_ambient = now + 60 + i64::from(kiln_javamath::random::RandomSource::next_int_bounded(&mut rng, 40));
        ambient_short = true;
    }
    if now % 80 == 0 {
        level.effect(Effect::Sound { pos, sound: "minecraft:block.conduit.ambient", volume: 1.0, pitch: 1.0 });
    }
    if ambient_short {
        level.effect(Effect::Sound { pos, sound: "minecraft:block.conduit.ambient.short", volume: 1.0, pitch: 1.0 });
    }
}

/// `applyEffects`: Conduit Power for the players in water or rain within `size / 7 * 16` blocks.
fn apply_effects(level: &RegionLevel, players: &mut [&mut Player], pos: BlockPos, size: usize) {
    let range = (size / 7 * 16) as f64;
    let Some(power) = crate::effects::effect_id("minecraft:conduit_power") else { return };
    // `new AABB(pos).inflate(range).expandTowards(0, level height, 0)`.
    let lo = [pos.x as f64 - range, pos.y as f64 - range, pos.z as f64 - range];
    let hi = [pos.x as f64 + 1.0 + range, pos.y as f64 + 1.0 + range + level.env.height as f64, pos.z as f64 + 1.0 + range];
    for p in players.iter_mut().filter(|p| !p.dead && !p.disconnected) {
        let h = p.dimensions().1 as f64;
        let (min, max) = ([p.pos[0] - 0.3, p.pos[1], p.pos[2] - 0.3], [p.pos[0] + 0.3, p.pos[1] + h, p.pos[2] + 0.3]);
        if !(0..3).all(|i| min[i] < hi[i] && max[i] > lo[i]) {
            continue;
        }
        let at = BlockPos::new(p.pos[0].floor() as i32, p.pos[1].floor() as i32, p.pos[2].floor() as i32);
        let (dx, dy, dz) = ((pos.x - at.x) as i64, (pos.y - at.y) as i64, (pos.z - at.z) as i64);
        // `BlockPos.closerThan(player.blockPosition(), range)`, then `isInWaterOrRain`.
        if ((dx * dx + dy * dy + dz * dz) as f64) < range * range && (p.was_touching_water || crate::weather::in_rain(level.cells, level.env, p.pos, p.pos[1] + h)) {
            p.add_effect(crate::effects::Effect::new(power, 260, 0, true, true, true));
        }
    }
}

/// `updateAndAttackTarget`.
fn update_and_attack_target(
    level: &mut RegionLevel,
    entities: &mut Entities,
    players: &mut [&mut Player],
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<crate::health::Death>,
    pos: BlockPos,
    can_destroy: bool,
) {
    let old = level.blocks.containers.get(pos).and_then(|c| c.conduit.as_deref()).and_then(|s| s.target);
    let new = update_destroy_target(level, entities, pos, old, can_destroy);
    if let Some(id) = new.and_then(|u| entities.list.iter().find(|e| !e.removed && e.uuid == u).map(|e| (e.id, e.pos))) {
        let (id, at) = id;
        if let Some(pkt) = crate::blocks::sound_packet_at("minecraft:block.conduit.attack.target", at, 1.0, 1.0, level.env, 7) {
            level.out.packets.push((at, 16.0, pkt));
        }
        crate::entities::hurt_by_block(entities, level, players, spawns, deaths, id, kiln_entity::level::DamageKind::Magic, 4.0);
    }
    if new != old {
        if let Some(st) = level.blocks.containers.get_mut(pos).and_then(|c| c.conduit.as_deref_mut()) {
            st.target = new;
        }
        crate::campfire::mark_updated(level, pos);
    }
}

/// The living, hostile, wet mob `uuid` if it is alive and within 8 blocks of `pos` (`closerThan` on its block position).
fn target_entity<'a>(entities: &'a Entities, pos: BlockPos, uuid: Uuid) -> Option<&'a crate::entities::Entity> {
    let e = entities.list.iter().find(|e| !e.removed && e.uuid == uuid)?;
    let alive = e.phys.as_deref().is_some_and(|p| p.is_alive());
    let at = BlockPos::new(e.pos[0].floor() as i32, e.pos[1].floor() as i32, e.pos[2].floor() as i32);
    let (dx, dy, dz) = ((pos.x - at.x) as i64, (pos.y - at.y) as i64, (pos.z - at.z) as i64);
    (alive && ((dx * dx + dy * dy + dz * dz) as f64) < 64.0).then_some(e)
}

/// `updateDestroyTarget` and `selectNewTarget`.
fn update_destroy_target(level: &RegionLevel, entities: &Entities, pos: BlockPos, target: Option<Uuid>, can_destroy: bool) -> Option<Uuid> {
    if !can_destroy {
        return None;
    }
    match target {
        None => select_new_target(level, entities, pos),
        Some(u) => target_entity(entities, pos, u).map(|_| u),
    }
}

/// `selectNewTarget`: a random one of the hostile mobs in water or rain in the 8-block box (the list is in entity id order here).
fn select_new_target(level: &RegionLevel, entities: &Entities, pos: BlockPos) -> Option<Uuid> {
    let (lo, hi) = ([pos.x as f64 - 8.0, pos.y as f64 - 8.0, pos.z as f64 - 8.0], [pos.x as f64 + 9.0, pos.y as f64 + 9.0, pos.z as f64 + 9.0]);
    let enemies = kiln_entity::mob::kinds::snow_golem::enemies();
    let list: Vec<&crate::entities::Entity> = entities
        .list
        .iter()
        .filter(|e| !e.removed && enemies.contains(&e.kind.name) && e.phys.as_deref().is_some_and(|p| p.is_alive()))
        .filter(|e| {
            let (min, max, _) = e.body();
            (0..3).all(|i| min[i] < hi[i] && max[i] > lo[i])
        })
        .filter(|e| mob_in_water_or_rain(level, e))
        .collect();
    if list.is_empty() {
        return None;
    }
    let mut rng = crate::container::pos_random(level, pos, 0x636f6e74);
    let i = kiln_javamath::random::RandomSource::next_int_bounded(&mut rng, list.len() as i32) as usize;
    Some(list[i].uuid)
}
