//! Bell block entities (`BellBlockEntity`): a bell that was hit shakes for 50 ticks; the living
//! things within 48 blocks (listed when it rings, and again if the last ring was over 60 ticks
//! ago) are the ones that hear it (the villagers within 32 hide) and, 5 ticks in, with raiders
//! among them within 32, it resonates: 40 ticks later every raider within 48 blocks glows.
//! The list is of this region's entities.

use crate::blocks::RegionLevel;
use crate::container::BeKind;
use crate::entities::Entities;
use kiln_blocks::{BlockPos, Direction, Effect};
use kiln_entity::mob::brain::village::closer_to_center_than;
use kiln_entity::mob::brain::{Mem, Val};
use kiln_world::ChunkPos;

/// `BellBlockEntity`'s fields.
#[derive(Debug, Clone, Default)]
pub(crate) struct BellState {
    pub ticks: i32,
    pub shaking: bool,
    pub click_direction: Option<Direction>,
    pub last_ring: i64,
    /// The entities listed at the last ring.
    pub nearby: Option<Vec<i32>>,
    pub resonating: bool,
    pub resonation_ticks: i32,
}

/// `BellBlockEntity.onHit`.
pub(crate) fn on_hit(level: &mut RegionLevel, pos: BlockPos, dir: Direction) -> bool {
    let Some(b) = level.blocks.containers.get_mut(pos).and_then(|c| c.bell.as_deref_mut()) else { return false };
    b.click_direction = Some(dir);
    if b.shaking {
        b.ticks = 0;
    } else {
        b.shaking = true;
    }
    true
}

/// `BellBlockEntity.triggerEvent(1, dir)` (the list of what hears it is made in
/// [`requests`], which has the entities).
pub(crate) fn trigger_event(level: &mut RegionLevel, pos: BlockPos, dir: Direction) -> bool {
    let Some(b) = level.blocks.containers.get_mut(pos).and_then(|c| c.bell.as_deref_mut()) else { return false };
    b.resonation_ticks = 0;
    b.click_direction = Some(dir);
    b.ticks = 0;
    b.shaking = true;
    level.out.bell_events.push(pos);
    true
}

fn in_tag(type_name: &str, tag: &str) -> bool {
    let Some(id) = kiln_data::builtin_id("minecraft:entity_type", type_name) else { return false };
    kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == "minecraft:entity_type")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .is_some_and(|(_, ids)| ids.contains(&id))
}

fn at(e: &crate::entities::Entity) -> kiln_entity::math::Vec3 {
    kiln_entity::math::Vec3::new(e.pos[0], e.pos[1], e.pos[2])
}

/// `BellBlockEntity.updateEntities`: for the rings the block events just ran.
pub(crate) fn requests(level: &mut RegionLevel, entities: &mut Entities) {
    if level.out.bell_events.is_empty() {
        return;
    }
    let now = level.env.game_time;
    for pos in std::mem::take(&mut level.out.bell_events) {
        let Some(b) = level.blocks.containers.get_mut(pos).and_then(|c| c.bell.as_deref_mut()) else { continue };
        if now > b.last_ring + 60 || b.nearby.is_none() {
            b.last_ring = now;
            // `getEntitiesOfClass(LivingEntity, new AABB(pos).inflate(48))`.
            let (lo, hi) = ([pos.x as f64 - 48.0, pos.y as f64 - 48.0, pos.z as f64 - 48.0], [pos.x as f64 + 49.0, pos.y as f64 + 49.0, pos.z as f64 + 49.0]);
            b.nearby = Some(
                entities
                    .list
                    .iter()
                    .filter(|e| !e.removed && e.phys.as_deref().is_some_and(|p| kiln_entity::mob::data(p).is_some()))
                    .filter(|e| {
                        let (min, max, _) = e.body();
                        (0..3).all(|i| min[i] < hi[i] && max[i] > lo[i])
                    })
                    .map(|e| e.id)
                    .collect(),
            );
        }
        let ids = b.nearby.clone().unwrap_or_default();
        let center = kiln_entity::math::BlockPos::new(pos.x, pos.y, pos.z);
        for id in ids {
            let Ok(i) = entities.list.binary_search_by_key(&id, |e| e.id) else { continue };
            let e = &mut entities.list[i];
            if e.removed || !closer_to_center_than(center, at(e), 32.0) {
                continue;
            }
            // `getBrain().setMemory(HEARD_BELL_TIME, gameTime)`.
            if let Some(phys) = e.phys.as_deref_mut()
                && let Some(m) = kiln_entity::mob::data_mut(phys)
                && let Some(brain) = m.brain.as_mut()
                && brain.st.mem.is_registered(Mem::HeardBellTime)
            {
                brain.st.mem.set(Mem::HeardBellTime, Val::Long(now));
            }
        }
    }
}

/// `BellBlockEntity.serverTick` of every bell in a ticking chunk.
pub(crate) fn tick(level: &mut RegionLevel, entities: &mut Entities, ticking: &crate::blocks::Ticking) {
    let bells: Vec<BlockPos> = level
        .blocks
        .containers
        .map
        .iter()
        .filter(|(p, c)| c.kind == BeKind::Bell && ticking.contains(ChunkPos::of_block(p.x, p.z)))
        .map(|(p, _)| *p)
        .collect();
    for pos in bells {
        let center = kiln_entity::math::BlockPos::new(pos.x, pos.y, pos.z);
        let Some(b) = level.blocks.containers.get_mut(pos).and_then(|c| c.bell.as_deref_mut()) else { continue };
        if b.shaking {
            b.ticks += 1;
        }
        if b.ticks >= 50 {
            b.shaking = false;
            b.ticks = 0;
        }
        let nearby = b.nearby.clone().unwrap_or_default();
        let live = |entities: &Entities, id: i32, range: f64| -> bool {
            let Ok(i) = entities.list.binary_search_by_key(&id, |e| e.id) else { return false };
            let e = &entities.list[i];
            !e.removed && e.phys.as_deref().is_some_and(|p| p.is_alive()) && closer_to_center_than(center, at(e), range) && in_tag(e.kind.name, "minecraft:raiders")
        };
        let mut resonate = false;
        if b.ticks >= 5 && b.resonation_ticks == 0 && nearby.iter().any(|&id| live(entities, id, 32.0)) {
            b.resonating = true;
            resonate = true;
        }
        let mut glow = false;
        if b.resonating {
            if b.resonation_ticks < 40 {
                b.resonation_ticks += 1;
            } else {
                glow = true;
                b.resonating = false;
            }
        }
        if resonate {
            level.effect(Effect::Sound { pos, sound: "minecraft:block.bell.resonate", volume: 1.0, pitch: 1.0 });
        }
        if glow {
            // `makeRaidersGlow`: Glowing for 60 ticks.
            let Some(id) = crate::effects::effect_id("minecraft:glowing") else { continue };
            for eid in nearby {
                if !live(entities, eid, 48.0) {
                    continue;
                }
                if let Ok(i) = entities.list.binary_search_by_key(&eid, |e| e.id)
                    && let Some(phys) = entities.list[i].phys.as_deref_mut()
                {
                    phys.pending_effects.push(crate::effects::Effect::simple(id, 60, 0));
                }
            }
        }
    }
}
