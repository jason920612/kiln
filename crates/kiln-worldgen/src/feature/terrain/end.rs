//! Simple End and void features: `EndIslandFeature`, `EndPlatformFeature` and
//! `VoidStartPlatformFeature`.

use super::ceil;
use crate::block_facts::Dir;
use crate::blocks::{is_block, same_block, state, with_prop};
use crate::vtags;
use crate::Error;
use crate::json::Json;
use kiln_proto::nbt::Tag;
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use kiln_javamath::math::{floor, floor_f32};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use std::f64::consts::PI;

/// `EndIslandFeature.place`: stacked shrinking discs of end stone.
pub fn end_island(r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
    end_island_with(random, origin, &mut |p, s| r.set_block(p, s));
    true
}

/// The blocks of [`end_island`] (it reads nothing), handed to `set` in placement order.
pub fn end_island_with(random: &mut impl RandomSource, origin: BlockPos, set: &mut dyn FnMut(BlockPos, u16)) {
    let mut radius = random.next_int_bounded(3) as f32 + 4.0;
    let mut y = 0;
    while radius > 0.5 {
        for x in floor_f32(-radius)..=ceil(radius) {
            for z in floor_f32(-radius)..=ceil(radius) {
                if ((x * x + z * z) as f32) <= (radius + 1.0) * (radius + 1.0) {
                    set(origin.offset(x, y, z), state::END_STONE);
                }
            }
        }
        radius -= random.next_int_bounded(2) as f32 + 0.5;
        y -= 1;
    }
}

/// `EndGatewayFeature` configuration: the exit portal position and whether teleports are
/// exact.
#[derive(Debug)]
pub struct EndGateway {
    exit: Option<BlockPos>,
    exact: bool,
}

impl EndGateway {
    pub fn parse(json: &Json) -> Result<EndGateway, Error> {
        let exit = match json.get("exit") {
            None => None,
            Some(v) => match v.as_array() {
                Some([x, y, z]) => Some(BlockPos::new(
                    x.as_i32().ok_or_else(|| Error::Invalid("bad exit".into()))?,
                    y.as_i32().ok_or_else(|| Error::Invalid("bad exit".into()))?,
                    z.as_i32().ok_or_else(|| Error::Invalid("bad exit".into()))?,
                )),
                _ => return Err(Error::Invalid("bad exit".into())),
            },
        };
        let exact = json.get("exact").and_then(Json::as_bool).ok_or_else(|| Error::Invalid("missing exact".into()))?;
        Ok(EndGateway { exit, exact })
    }

    /// `EndGatewayFeature.place`: the gateway block in a bedrock frame, cleared around.
    pub fn place(&self, r: &mut Region, origin: BlockPos) -> bool {
        end_gateway_with(origin, &mut |p, s| r.set_block(p, s));
        if let Some(exit) = self.exit
            && let Some(tag) = r.block_entity_mut(origin)
        {
            *tag = end_gateway_entity(Some((exit, self.exact)));
        }
        true
    }
}

/// The blocks of `EndGatewayFeature.place` around `origin` (it reads nothing): the gateway,
/// bedrock above and below and on the sides of the middle layers, air elsewhere in the 3x5x3.
pub fn end_gateway_with(origin: BlockPos, set: &mut dyn FnMut(BlockPos, u16)) {
    for p in super::between_closed(origin.offset(-1, -2, -1), origin.offset(1, 2, 1)) {
        let (same_x, same_y, same_z) = (p.x == origin.x, p.y == origin.y, p.z == origin.z);
        let two_off = (p.y - origin.y).abs() == 2;
        let s = if same_x && same_y && same_z {
            state::END_GATEWAY
        } else if same_y {
            state::AIR
        } else if (two_off && same_x && same_z) || ((same_x || same_z) && !two_off) {
            state::BEDROCK
        } else {
            state::AIR
        };
        set(p, s);
    }
}

/// The saved `TheEndGatewayBlockEntity` a gateway feature leaves (`Age` 0; `exit_portal` and
/// `ExactTeleport` when the feature knows the exit).
pub fn end_gateway_entity(exit: Option<(BlockPos, bool)>) -> Tag {
    let mut fields = vec![("id".into(), Tag::String("minecraft:end_gateway".into())), ("Age".into(), Tag::Long(0))];
    if let Some((exit, exact)) = exit {
        fields.push(("exit_portal".into(), Tag::IntArray(vec![exit.x, exit.y, exit.z])));
        fields.push(("ExactTeleport".into(), Tag::Byte(exact as i8)));
    } else {
        fields.push(("ExactTeleport".into(), Tag::Byte(0)));
    }
    Tag::Compound(fields)
}

/// `EndSpikeFeature.EndSpike`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndSpike {
    pub center_x: i32,
    pub center_z: i32,
    pub radius: i32,
    pub height: i32,
    pub guarded: bool,
}

/// `EndSpikeFeature`: obsidian pillars (from the configuration, or the level's ten), iron
/// bar cages on guarded ones, and a bedrock and fire top where the end crystal stands.
///
/// The crystal (bottom shown, random yaw) goes to the chunk's entity list.
#[derive(Debug)]
pub struct EndSpikes {
    spikes: Vec<EndSpike>,
}

impl EndSpikes {
    pub fn parse(json: &Json) -> Result<EndSpikes, Error> {
        let bad = || Error::Invalid("bad end spike".into());
        let spikes = match json.get("spikes").and_then(Json::as_array) {
            None => Vec::new(),
            Some(list) => list
                .iter()
                .map(|s| {
                    let int = |k: &str, d: i32| s.get(k).map_or(Some(d), Json::as_i32).ok_or_else(bad);
                    Ok(EndSpike {
                        center_x: int("centerX", 0)?,
                        center_z: int("centerZ", 0)?,
                        radius: int("radius", 0)?,
                        height: int("height", 0)?,
                        guarded: s.get("guarded").map_or(Some(false), Json::as_bool).ok_or_else(bad)?,
                    })
                })
                .collect::<Result<_, Error>>()?,
        };
        Ok(EndSpikes { spikes })
    }

    pub fn place(&self, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let spikes = if self.spikes.is_empty() { spikes_for_seed(r.seed()) } else { self.spikes.clone() };
        for s in spikes {
            if s.center_x >> 4 == origin.x >> 4 && s.center_z >> 4 == origin.z >> 4 {
                place_spike(r, random, s);
            }
        }
        true
    }
}

/// `EndSpikeFeature.getSpikesForLevel` and `SpikeCacheLoader.load`.
pub fn spikes_for_seed(seed: i64) -> Vec<EndSpike> {
    let key = LegacyRandom::new(seed).next_long() & 0xFFFF;
    let mut random = LegacyRandom::new(key);
    let mut order: Vec<i32> = (0..10).collect();
    for i in (2..=order.len()).rev() {
        let j = random.next_int_bounded(i as i32) as usize;
        order.swap(i - 1, j);
    }
    (0..10)
        .map(|i| {
            let angle = 2.0 * (-PI + 0.314_159_265_358_979_3 * i as f64);
            let n = order[i];
            EndSpike {
                center_x: floor(42.0 * kiln_javamath::trig::cos(angle)),
                center_z: floor(42.0 * kiln_javamath::trig::sin(angle)),
                radius: 2 + n / 3,
                height: 76 + n * 3,
                guarded: n == 1 || n == 2,
            }
        })
        .collect()
}

/// `EndSpikeFeature.placeSpike`.
fn place_spike(r: &mut Region, random: &mut WorldgenRandom, s: EndSpike) {
    let min_y = r.min_y();
    spike_with(s, min_y, &mut |p, state| r.set_block(p, state));
    let yaw = random.next_float() * 360.0;
    let crystal = BlockPos::new(s.center_x, s.height + 1, s.center_z);
    let (x, y, z) = (crystal.x as f64 + 0.5, crystal.y as f64, crystal.z as f64 + 0.5);
    r.add_entity(x, z, crate::feature::entity_tag("minecraft:end_crystal", [x, y, z], yaw, vec![("ShowBottom".into(), Tag::Byte(1))]));
    r.set_block(crystal.below(), state::BEDROCK);
    // `BaseFireBlock.getState` on bedrock: plain fire, sturdy floor so no side flags.
    r.set_block(crystal, state::FIRE);
}

/// The blocks of `placeSpike` in its order (the obsidian column, air above 65 around it, the
/// iron bar cage of a guarded one), before its crystal; `set` gets each write.
pub fn spike_with(s: EndSpike, min_y: i32, set: &mut dyn FnMut(BlockPos, u16)) {
    let rad = s.radius;
    let from = BlockPos::new(s.center_x - rad, min_y, s.center_z - rad);
    let to = BlockPos::new(s.center_x + rad, s.height + 10, s.center_z + rad);
    for p in super::between_closed(from, to) {
        let (dx, dz) = ((p.x - s.center_x) as f64, (p.z - s.center_z) as f64);
        if dx * dx + dz * dz <= (rad * rad + 1) as f64 && p.y < s.height {
            set(p, state::OBSIDIAN);
        } else if p.y > 65 {
            set(p, state::AIR);
        }
    }
    if s.guarded {
        for dx in -2i32..=2 {
            for dz in -2i32..=2 {
                for dy in 0..=3 {
                    let (edge_x, edge_z, top) = (dx.abs() == 2, dz.abs() == 2, dy == 3);
                    if !(edge_x || edge_z || top) {
                        continue;
                    }
                    let ns = dx == -2 || dx == 2 || top;
                    let ew = dz == -2 || dz == 2 || top;
                    let mut bars = state::IRON_BARS;
                    bars = with_prop(bars, "north", bool_str(ns && dz != -2));
                    bars = with_prop(bars, "south", bool_str(ns && dz != 2));
                    bars = with_prop(bars, "west", bool_str(ew && dx != -2));
                    bars = with_prop(bars, "east", bool_str(ew && dx != 2));
                    set(BlockPos::new(s.center_x + dx, s.height + dy, s.center_z + dz), bars);
                }
            }
        }
    }
}

/// `ChorusPlantFeature.place`: `ChorusFlowerBlock.generatePlant(level, origin, random, 8)` on
/// a supporting block.
pub fn chorus_plant(r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
    if !r.is_air(origin) || !vtags::is(r.get(origin.below()), "supports_chorus_plant") {
        return false;
    }
    set_chorus_plant(r, origin);
    grow_chorus(r, origin, random, origin, 8, 0);
    true
}

/// `ChorusFlowerBlock.growTreeRecursive`.
fn grow_chorus(r: &mut Region, p: BlockPos, random: &mut WorldgenRandom, root: BlockPos, max_spread: i32, depth: i32) {
    let mut height = random.next_int_bounded(4) + 1;
    if depth == 0 {
        height += 1;
    }
    for i in 0..height {
        let q = p.above_n(i + 1);
        if !all_neighbors_empty(r, q, None) {
            return;
        }
        set_chorus_plant(r, q);
        set_chorus_plant(r, q.below());
    }
    let mut branched = false;
    if depth < 4 {
        let mut branches = random.next_int_bounded(4);
        if depth == 0 {
            branches += 1;
        }
        for _ in 0..branches {
            let d = Dir::HORIZONTAL[random.next_int_bounded(4) as usize];
            let q = p.above_n(height).relative(d);
            if (q.x - root.x).abs() < max_spread
                && (q.z - root.z).abs() < max_spread
                && r.is_air(q)
                && r.is_air(q.below())
                && all_neighbors_empty(r, q, Some(d.opposite()))
            {
                branched = true;
                set_chorus_plant(r, q);
                set_chorus_plant(r, q.relative(d.opposite()));
                grow_chorus(r, q, random, root, max_spread, depth + 1);
            }
        }
    }
    if !branched {
        r.set(p.above_n(height), with_prop(state::CHORUS_FLOWER, "age", "5"), 2);
    }
}

/// `ChorusFlowerBlock.allNeighborsEmpty`: every horizontal neighbour but `except` is air.
fn all_neighbors_empty(r: &mut Region, p: BlockPos, except: Option<Dir>) -> bool {
    Dir::HORIZONTAL.into_iter().filter(|&d| Some(d) != except).all(|d| r.is_air(p.relative(d)))
}

/// Places a chorus plant with `ChorusPlantBlock.getStateWithConnections`.
fn set_chorus_plant(r: &mut Region, p: BlockPos) {
    let connects = |s: u16| is_block(s, "minecraft:chorus_plant") || is_block(s, "minecraft:chorus_flower");
    let below = r.get(p.below());
    let mut s = with_prop(
        state::CHORUS_PLANT,
        "down",
        bool_str(connects(below) || vtags::is(below, "supports_chorus_plant")),
    );
    for d in [Dir::Up, Dir::North, Dir::East, Dir::South, Dir::West] {
        let n = r.get(p.relative(d));
        s = with_prop(s, d.name(), bool_str(connects(n)));
    }
    r.set(p, s, 2);
}

fn bool_str(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

/// `EndPlatformFeature.createEndPlatform(level, origin, false)`: a 5×5 obsidian floor with
/// air above.
pub fn end_platform(r: &mut Region, origin: BlockPos) -> bool {
    for z in -2..=2 {
        for x in -2..=2 {
            for y in -1..3 {
                let p = origin.offset(x, y, z);
                let s = if y == -1 { state::OBSIDIAN } else { state::AIR };
                if !same_block(r.get(p), s) {
                    r.set_block(p, s);
                }
            }
        }
    }
    true
}

/// `VoidStartPlatformFeature.place`: stone within 16 blocks (chessboard) of (8, y + 3, 8),
/// cobblestone at its center, in the chunks around the origin chunk.
pub fn void_start_platform(r: &mut Region, origin: BlockPos) -> bool {
    let (cx, cz) = (origin.x >> 4, origin.z >> 4);
    if cx.abs().max(cz.abs()) > 1 {
        return true;
    }
    let center = BlockPos::new(8, origin.y + 3, 8);
    for z in cz * 16..cz * 16 + 16 {
        for x in cx * 16..cx * 16 + 16 {
            if (center.x - x).abs().max((center.z - z).abs()) > 16 {
                continue;
            }
            let p = BlockPos::new(x, center.y, z);
            r.set(p, if p == center { state::COBBLESTONE } else { state::STONE }, 2);
        }
    }
    true
}
