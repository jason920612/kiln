//! Differential tests against vanilla 26.3: replays the scenarios recorded by
//! tools/EntityVectors.java (`python tools/entity_parity.py`) and compares every entity's state
//! after every tick, bit for bit.
//!
//! Vectors: `$KILN_ENTITY_VECTORS`, else `<KILN_WORK or workspace/work>/wp4-entities/vectors.jsonl`;
//! the test is skipped when they are absent. `KILN_PARITY_FILTER` selects scenarios by name.

use kiln_entity::entity::{Entity, EntityKind};
use kiln_entity::level::{EntityFilter, EntityLevel, Event};
use kiln_entity::math::{Aabb, BlockPos};
use kiln_entity::{falling_block, item, player, tnt, xp_orb};
use kiln_item::ItemStack;
use kiln_javamath::random::LegacyRandom;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

struct Slot {
    entity: Option<Entity>,
    /// Section key and insertion sequence, for vanilla's entity iteration order.
    section: (i32, i64),
    seq: u64,
}

struct TestLevel {
    blocks: HashMap<BlockPos, u16>,
    slots: Vec<Slot>,
    index: HashMap<i32, usize>,
    random: LegacyRandom,
    next_id: i32,
    next_seq: u64,
    events: Vec<Event>,
    spawned: Vec<Entity>,
}

fn section_of(e: &Entity) -> (i32, i64) {
    let p = e.block_position();
    let (sx, sy, sz) = (p.x >> 4, p.y >> 4, p.z >> 4);
    // SectionPos.asLong order within one x: z (22 bits), then y (20 bits), unsigned fields.
    (sx, (((sz as i64) & 0x3F_FFFF) << 20) | ((sy as i64) & 0xF_FFFF))
}

impl TestLevel {
    fn new(seed: i64) -> Self {
        TestLevel {
            blocks: HashMap::new(),
            slots: Vec::new(),
            index: HashMap::new(),
            random: LegacyRandom::new(seed),
            next_id: 1_000_000,
            next_seq: 0,
            events: Vec::new(),
            spawned: Vec::new(),
        }
    }

    fn insert(&mut self, e: Entity) {
        let section = section_of(&e);
        self.index.insert(e.id, self.slots.len());
        self.slots.push(Slot { entity: Some(e), section, seq: self.next_seq });
        self.next_seq += 1;
    }

    fn resection(&mut self, i: usize) {
        let s = &mut self.slots[i];
        if let Some(e) = &s.entity {
            let now = section_of(e);
            if now != s.section {
                s.section = now;
                s.seq = self.next_seq;
                self.next_seq += 1;
            }
        }
    }
}

impl EntityLevel for TestLevel {
    fn block(&self, pos: BlockPos) -> u16 {
        // The harness world is superflat with one bedrock layer at the bottom.
        let floor = if pos.y == -64 { kiln_data::blocks::default_state::BEDROCK } else { 0 };
        self.blocks.get(&pos).copied().unwrap_or(floor)
    }

    fn set_block(&mut self, pos: BlockPos, state: u16, _flags: u32) -> bool {
        let old = self.blocks.insert(pos, state).unwrap_or(0);
        old != state
    }

    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }

    fn game_time(&self) -> i64 {
        0
    }

    fn min_y(&self) -> i32 {
        -64
    }

    fn entities_in(&self, area: &Aabb, filter: EntityFilter, exclude: i32) -> Vec<i32> {
        let mut found: Vec<((i32, i64), u64, i32)> = self
            .slots
            .iter()
            .filter_map(|s| {
                let e = s.entity.as_ref()?;
                let wanted = match filter {
                    EntityFilter::Any => true,
                    EntityFilter::Item => matches!(e.kind, EntityKind::Item(_)),
                    EntityFilter::ExperienceOrb => matches!(e.kind, EntityKind::ExperienceOrb(_)),
                    EntityFilter::Living => matches!(e.kind, EntityKind::Other { .. }),
                };
                (wanted && e.id != exclude && e.is_alive() && e.bounding_box().intersects(area))
                    .then_some((s.section, s.seq, e.id))
            })
            .collect();
        found.sort();
        found.into_iter().map(|(_, _, id)| id).collect()
    }

    fn entity_mut(&mut self, id: i32) -> Option<&mut Entity> {
        let i = *self.index.get(&id)?;
        self.slots[i].entity.as_mut()
    }

    fn entity(&self, id: i32) -> Option<&Entity> {
        let i = *self.index.get(&id)?;
        self.slots[i].entity.as_ref()
    }

    fn add_entity(&mut self, entity: Entity) {
        self.spawned.push(entity);
    }

    fn fresh_seed(&mut self) -> i64 {
        self.next_id as i64 * 0x5DEE_CE66
    }

    fn next_entity_id(&mut self) -> i32 {
        self.next_id += 1;
        self.next_id
    }

    fn emit(&mut self, event: Event) {
        self.events.push(event);
    }
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn vec3(v: &Value) -> kiln_entity::math::Vec3 {
    kiln_entity::math::Vec3::new(f(&v[0]), f(&v[1]), f(&v[2]))
}

fn spawn(spec: &Value) -> Entity {
    let id = spec["id"].as_i64().unwrap() as i32;
    let seed = spec["seed"].as_i64().unwrap();
    let int = |k: &str, d: i64| spec.get(k).and_then(Value::as_i64).unwrap_or(d) as i32;
    let kind = spec["kind"].as_str().unwrap();
    let mut e = match kind {
        "item" => {
            let name = spec.get("item").and_then(Value::as_str).unwrap_or("minecraft:stone");
            let stack = ItemStack::of(name, int("count", 1)).unwrap_or_else(|| panic!("unknown item {name}"));
            let mut data = item::ItemData::new(stack);
            data.pickup_delay = int("pickup_delay", 10);
            data.age = int("age", 0);
            Entity::new("minecraft:item", id, 0, EntityKind::Item(data), seed)
        }
        "tnt" => Entity::new(
            "minecraft:tnt",
            id,
            0,
            EntityKind::Tnt(tnt::TntData { fuse: int("fuse", 80), ..tnt::TntData::new() }),
            seed,
        ),
        "falling_block" => {
            let state = spec["block_id"].as_i64().unwrap_or(0) as u16;
            Entity::new(
                "minecraft:falling_block",
                id,
                0,
                EntityKind::FallingBlock(falling_block::FallingBlockData {
                    state,
                    time: int("time", 0),
                    drop_item: spec.get("drop_item").and_then(Value::as_bool).unwrap_or(true),
                    cancel_drop: spec.get("cancel_drop").and_then(Value::as_bool).unwrap_or(false),
                    hurt_entities: spec.get("hurt_per_distance").is_some(),
                    fall_damage_max: int("hurt_max", 40),
                    fall_damage_per_distance: spec.get("hurt_per_distance").map_or(0.0, f) as f32,
                }),
                seed,
            )
        }
        "experience_orb" => Entity::new(
            "minecraft:experience_orb",
            id,
            0,
            EntityKind::ExperienceOrb(xp_orb::OrbData { count: int("count", 1), age: int("age", 0), ..xp_orb::OrbData::new(int("value", 1)) }),
            seed,
        ),
        "player" => {
            let shift = spec.get("shift").and_then(Value::as_bool).unwrap_or(false);
            let mut p = player::new(id, 0, vec3(&spec["pos"]), if shift { 1.5 } else { 1.8 }, 0.6);
            p.shift_key_down = shift;
            if let EntityKind::Player(d) = &mut p.kind {
                d.flying = spec.get("flying").and_then(Value::as_bool).unwrap_or(false);
            }
            p
        }
        other => panic!("unknown kind {other}"),
    };
    e.set_pos(vec3(&spec["pos"]));
    e.delta = vec3(&spec["motion"]);
    e.y_rot = f(&spec["yaw"]) as f32;
    if spec.get("no_gravity").is_some() {
        e.no_gravity = true;
    }
    if let Some(v) = spec.get("fire") {
        e.remaining_fire_ticks = v.as_i64().unwrap() as i32;
    }
    if let Some(v) = spec.get("fall_distance") {
        e.fall_distance = f(v);
    }
    e
}

/// The recorded state vector of an entity (see EntityVectors.state).
fn state(e: &Entity) -> Vec<f64> {
    let p = e.position();
    let v = e.delta;
    let b = |x: bool| if x { 1.0 } else { 0.0 };
    let mut out = vec![
        e.id as f64,
        p.x,
        p.y,
        p.z,
        v.x,
        v.y,
        v.z,
        b(e.on_ground),
        b(e.horizontal_collision),
        b(e.vertical_collision),
        e.fall_distance,
        b(e.is_removed()),
        e.remaining_fire_ticks as f64,
        e.air_supply as f64,
    ];
    match &e.kind {
        EntityKind::Item(d) => {
            out.extend([d.stack.count() as f64, d.age as f64, d.pickup_delay as f64, d.health as f64]);
        }
        EntityKind::Tnt(d) => out.push(d.fuse as f64),
        EntityKind::FallingBlock(d) => out.extend([d.time as f64, d.state as f64]),
        EntityKind::ExperienceOrb(d) => out.extend([d.value as f64, d.count as f64, d.age as f64]),
        EntityKind::Player(_) | EntityKind::Other { .. } => {}
    }
    out
}

const FIELDS: &[&str] = &[
    "id", "x", "y", "z", "dx", "dy", "dz", "on_ground", "h_coll", "v_coll", "fall", "removed", "fire", "air", "k0", "k1", "k2", "k3",
];

/// Replays one scenario; `Err` describes the first mismatch.
fn replay(s: &Value) -> Result<(), String> {
    let mut level = TestLevel::new(s["level_seed"].as_i64().unwrap());
    for b in s["blocks"].as_array().unwrap() {
        let p = BlockPos::new(b[0].as_i64().unwrap() as i32, b[1].as_i64().unwrap() as i32, b[2].as_i64().unwrap() as i32);
        level.blocks.insert(p, b[3].as_u64().unwrap() as u16);
    }
    let mut moves: HashMap<i32, Vec<kiln_entity::math::Vec3>> = HashMap::new();
    for spec in s["entities"].as_array().unwrap() {
        let mut e = spawn(spec);
        if let Some(m) = spec.get("moves").and_then(Value::as_array) {
            moves.insert(e.id, m.iter().map(vec3).collect());
            if spec.get("on_ground").and_then(Value::as_bool).unwrap_or(false) {
                e.set_on_ground(&level, true);
            }
        }
        level.insert(e);
    }
    let trace = s["trace"].as_array().unwrap();
    let initial = level.slots.len();
    let mut seen_spawned: Vec<usize> = Vec::new();
    for (tick, expected) in trace.iter().enumerate() {
        let order: Vec<usize> = (0..level.slots.len()).collect();
        for i in order {
            let Some(mut e) = level.slots[i].entity.take() else { continue };
            if let Some(m) = moves.get(&e.id) {
                e.delta = m[tick];
                player::server_move(&mut level, &mut e, m[tick]);
            } else if !e.is_removed() {
                e.common_tick();
                e.tick(&mut level);
            }
            level.slots[i].entity = Some(e);
            level.resection(i);
        }
        for e in std::mem::take(&mut level.spawned) {
            level.insert(e);
        }
        let expected = expected.as_array().unwrap();
        for (k, want) in expected.iter().enumerate() {
            let want: Vec<f64> = want.as_array().unwrap().iter().map(f).collect();
            let Some(slot) = level.slots.get(k) else {
                return Err(format!("tick {tick}: vanilla has entity #{k} ({want:?}), kiln has none"));
            };
            let got = state(slot.entity.as_ref().unwrap());
            // Entities spawned during the run (drops, primed TNT) get fresh ids and, in vanilla,
            // velocities from an unseeded random: compare where and what they are when they appear.
            let spawned = k >= initial;
            if spawned && seen_spawned.contains(&k) {
                continue;
            }
            if spawned {
                seen_spawned.push(k);
            }
            if got.len() != want.len() {
                return Err(format!("tick {tick}: entity {k} state length {} vs {}", got.len(), want.len()));
            }
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                if spawned && matches!(i, 0 | 4 | 5 | 6) {
                    continue;
                }
                if g.to_bits() != w.to_bits() {
                    return Err(format!(
                        "tick {tick} entity {} field {}: kiln {g:?} vanilla {w:?}\n  kiln    {got:?}\n  vanilla {want:?}",
                        want[0], FIELDS[i]
                    ));
                }
            }
        }
        if level.slots.len() > expected.len() {
            return Err(format!("tick {tick}: kiln has {} entities, vanilla {}", level.slots.len(), expected.len()));
        }
    }
    if let Some(want) = s["final_blocks"].as_array() {
        for b in want {
            let p = BlockPos::new(b[0].as_i64().unwrap() as i32, b[1].as_i64().unwrap() as i32, b[2].as_i64().unwrap() as i32);
            let w = b[3].as_u64().unwrap() as u16;
            if level.block(p) != w {
                return Err(format!("final block at {p:?}: kiln {} vanilla {w}", level.block(p)));
            }
        }
        let nonair = level.blocks.values().filter(|&&v| v != 0).count();
        if nonair != want.len() {
            return Err(format!("final blocks: kiln has {nonair} non-air, vanilla {}", want.len()));
        }
    }
    Ok(())
}

fn vectors_path() -> PathBuf {
    if let Some(p) = std::env::var_os("KILN_ENTITY_VECTORS") {
        return PathBuf::from(p);
    }
    let work = std::env::var_os("KILN_WORK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
    work.join("wp4-entities/vectors.jsonl")
}

#[test]
fn vanilla_parity() {
    let path = vectors_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skipping: no vectors at {}", path.display());
        return;
    };
    let filter = std::env::var("KILN_PARITY_FILTER").ok();
    let (mut pass, mut fail) = (0, 0);
    let mut by_group: std::collections::BTreeMap<String, (u32, u32)> = Default::default();
    let mut failures = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let s: Value = serde_json::from_str(line).expect("vector line");
        let name = s["name"].as_str().unwrap().to_string();
        if filter.as_deref().is_some_and(|f| !name.contains(f)) {
            continue;
        }
        let group = name.split('/').next().unwrap().to_string();
        let result = if s.get("error").is_some() {
            Err(format!("vanilla error: {}", s["error"]))
        } else {
            std::panic::catch_unwind(|| replay(&s)).unwrap_or_else(|p| {
                Err(format!("panic: {}", p.downcast_ref::<String>().cloned().unwrap_or_else(|| {
                    p.downcast_ref::<&str>().map(|s| s.to_string()).unwrap_or_default()
                })))
            })
        };
        let g = by_group.entry(group).or_default();
        match result {
            Ok(()) => {
                pass += 1;
                g.0 += 1;
            }
            Err(msg) => {
                fail += 1;
                g.1 += 1;
                failures.push(format!("{name}: {msg}"));
            }
        }
    }
    for (g, (p, f)) in &by_group {
        eprintln!("{g:24} {p:5} pass {f:5} fail");
    }
    for msg in failures.iter().take(15) {
        eprintln!("FAIL {msg}");
    }
    eprintln!("parity: {pass} passed, {fail} failed");
    assert_eq!(fail, 0, "{fail} scenarios differ from vanilla");
}
