//! Block placements of the End that the dragon fight (not chunk generation) makes, as pure
//! functions the simulation applies block by block:
//!
//! - the exit portal (`EndPodiumFeature`, placed by `EnderDragonFight.spawnExitPortal`):
//!   [`exit_portal_origin`] finds where, [`end_podium_blocks`] says what;
//! - the end gateways (`EnderDragonFight.spawnNewGateway`): [`end_gateway_positions`] in the
//!   order vanilla spawns them, [`end_gateway_blocks`] and [`end_gateway_entity`] for each
//!   (`end_gateway_delayed`: no exit yet, not exact);
//! - the island a gateway teleport creates when it finds no land
//!   (`TheEndGatewayBlockEntity.findOrCreateValidTeleportPos`: `end_island` placed with
//!   `RandomSource.create(pos.asLong())`): [`end_island_blocks`].

use crate::blocks::{state, with_prop};
use crate::feature::terrain::end::{end_gateway_with, end_island_with};
use crate::pos::BlockPos;
use kiln_javamath::math::floor;
use kiln_javamath::random::LegacyRandom;

pub use crate::feature::terrain::end::end_gateway_entity;

/// `EnderDragonFight.END_PODIUM_LOCATION` (the fight's origin offset, `BlockPos.ZERO`).
pub const PODIUM_COLUMN: (i32, i32) = (0, 0);

/// `EnderDragonFight.spawnExitPortal`'s location search, the first time: one below
/// `MOTION_BLOCKING_NO_LEAVES` at (0, 0) (`surface_y` is that heightmap's value), then down
/// while the block there is bedrock and y > 63, then at least `min_y + 1`. Vanilla remembers
/// the result (`ExitPortalLocation`) and reuses it afterwards.
pub fn exit_portal_origin(surface_y: i32, min_y: i32, is_bedrock: impl Fn(BlockPos) -> bool) -> BlockPos {
    let mut p = BlockPos::new(PODIUM_COLUMN.0, surface_y - 1, PODIUM_COLUMN.1);
    while is_bedrock(p) && p.y > 63 {
        p = p.below();
    }
    p.at_y(p.y.max(min_y + 1))
}

/// One block of the exit portal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PodiumBlock {
    pub pos: BlockPos,
    pub state: u16,
    /// `dropPreviousAndSetBlock` (the active podium's end stone, air and portal): if the block
    /// there is not already this block, vanilla first destroys it with drops
    /// (`destroyBlock(pos, true)`), then sets the state.
    pub drop_previous: bool,
}

/// `EndPodiumFeature(active).place` at `origin` (from [`exit_portal_origin`]), in vanilla's
/// order: every position of the box (-4, -1, -4)..(4, 32, 4) within 3.5 blocks of the origin
/// (squared distance of block coordinates, y included): bedrock rim (within 2.5 below, beyond
/// 2.5 at the origin's level), end stone below, air above, portal (active) or air in the
/// middle; then the bedrock pillar (origin up to 3 above) and four wall torches around its
/// second block. Vanilla writes with `setBlock` flag 3; nothing is read except by
/// `drop_previous`.
pub fn end_podium_blocks(origin: BlockPos, active: bool) -> Vec<PodiumBlock> {
    let mut out = Vec::new();
    let mut put = |pos: BlockPos, state: u16, drop_previous: bool| out.push(PodiumBlock { pos, state, drop_previous });
    // `BlockPos.betweenClosed`: x fastest, then y, then z.
    for z in origin.z - 4..=origin.z + 4 {
        for y in origin.y - 1..=origin.y + 32 {
            for x in origin.x - 4..=origin.x + 4 {
                let p = BlockPos::new(x, y, z);
                let d = p.dist_sqr(origin);
                let inner = d < 2.5 * 2.5;
                if !inner && d >= 3.5 * 3.5 {
                    continue;
                }
                if p.y < origin.y {
                    if inner {
                        put(p, state::BEDROCK, false);
                    } else {
                        put(p, state::END_STONE, active);
                    }
                } else if p.y > origin.y {
                    put(p, state::AIR, active);
                } else if !inner {
                    put(p, state::BEDROCK, false);
                } else if active {
                    put(p, state::END_PORTAL, true);
                } else {
                    put(p, state::AIR, false);
                }
            }
        }
    }
    for dy in 0..4 {
        put(origin.above_n(dy), state::BEDROCK, false);
    }
    let torch = origin.above_n(2);
    for (facing, (dx, dz)) in [("north", (0, -1)), ("east", (1, 0)), ("south", (0, 1)), ("west", (-1, 0))] {
        put(torch.offset(dx, 0, dz), with_prop(state::WALL_TORCH, "facing", facing), false);
    }
    out
}

/// The 20 end gateway positions (radius 96 around the origin at y 75) in the order
/// `EnderDragonFight.spawnNewGateway` uses them: `init` shuffles the indices 0..20 with
/// `Util.shuffle` on a `SingleThreadedRandomSource(seed)` (the LCG), and each kill takes the
/// last remaining index. Index `i` is at `floor(96 cos(2(-pi + 0.15707963267948966 i)))`,
/// same with `sin` for z.
pub fn end_gateway_positions(seed: i64) -> Vec<BlockPos> {
    use kiln_javamath::random::RandomSource;
    let mut list: Vec<i32> = (0..20).collect();
    let mut random = LegacyRandom::new(seed);
    for i in (2..=list.len()).rev() {
        let j = random.next_int_bounded(i as i32) as usize;
        list.swap(i - 1, j);
    }
    list.iter()
        .rev()
        .map(|&i| {
            let angle = 2.0 * (-std::f64::consts::PI + 0.157_079_632_679_489_66 * i as f64);
            BlockPos::new(floor(96.0 * kiln_javamath::trig::cos(angle)), 75, floor(96.0 * kiln_javamath::trig::sin(angle)))
        })
        .collect()
}

/// `EndGatewayFeature.place` at `origin`: 45 blocks in vanilla's order (flag 3). The gateway
/// block at `origin` then gets [`end_gateway_entity`] (`exit`: the known exit and whether the
/// teleport is exact; `None` for the dragon fight's `end_gateway_delayed`).
pub fn end_gateway_blocks(origin: BlockPos) -> Vec<(BlockPos, u16)> {
    let mut out = Vec::new();
    end_gateway_with(origin, &mut |p, s| out.push((p, s)));
    out
}

/// The `end_island` feature at `origin` with `RandomSource.create(seed)` (a legacy LCG; the
/// gateway passes `pos.asLong()`, see [`BlockPos::as_long`]): end stone discs, in vanilla's
/// order (flag 3). It reads nothing.
pub fn end_island_blocks(origin: BlockPos, seed: i64) -> Vec<(BlockPos, u16)> {
    let mut out = Vec::new();
    end_island_with(&mut LegacyRandom::new(seed), origin, &mut |p, s| out.push((p, s)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn podium_shape() {
        let o = BlockPos::new(0, 64, 0);
        let active = end_podium_blocks(o, true);
        let portal = active.iter().filter(|b| b.state == state::END_PORTAL).count();
        // The 5x5 minus corners at the origin's level within 2.5 blocks: 21 placements (the
        // center is overwritten by the pillar afterwards).
        assert_eq!(portal, 21);
        let torches = active.iter().filter(|b| crate::blocks::is_block(b.state, "minecraft:wall_torch")).count();
        assert_eq!(torches, 4);
        let inactive = end_podium_blocks(o, false);
        assert_eq!(inactive.len(), active.len());
        assert!(inactive.iter().all(|b| b.state != state::END_PORTAL && !b.drop_previous));
    }

    #[test]
    fn gateways_are_a_ring_of_twenty() {
        let g = end_gateway_positions(12345);
        assert_eq!(g.len(), 20);
        for p in &g {
            let r = ((p.x * p.x + p.z * p.z) as f64).sqrt();
            assert!((94.0..=97.0).contains(&r), "{p:?}");
            assert_eq!(p.y, 75);
        }
        let mut sorted = g.clone();
        sorted.sort_by_key(|p| (p.x, p.z));
        sorted.dedup();
        assert_eq!(sorted.len(), 20);
    }
}
