//! Multiface blocks (glow lichen, sculk veins, ...): placement onto a face (`MultifaceBlock`)
//! and spreading to neighbouring faces (`MultifaceSpreader`).

use super::shape::can_attach_to;
use crate::block_facts::{Dir, fluid};
use crate::blocks::{is_air, is_block, prop, same_block, with_prop};
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use kiln_data::blocks_types::block_of;
use kiln_javamath::random::RandomSource;

/// `Util.shuffle`.
pub fn shuffle<T>(list: &mut [T], random: &mut WorldgenRandom) {
    for i in (2..=list.len()).rev() {
        let j = random.next_int_bounded(i as i32) as usize;
        list.swap(i - 1, j);
    }
}

/// `Direction.allShuffled`.
pub fn all_shuffled(random: &mut WorldgenRandom) -> [Dir; 6] {
    let mut d = Dir::ALL;
    shuffle(&mut d, random);
    d
}

/// `MultifaceBlock.hasFace`.
pub fn has_face(s: u16, d: Dir) -> bool {
    prop(s, d.name()) == Some("true")
}

/// `MultifaceBlock.isValidStateForPlacement` for a multiface block whose default state is
/// `block`.
pub fn is_valid_for_placement(block: u16, r: &mut Region, current: u16, p: BlockPos, d: Dir) -> bool {
    if prop(block, d.name()).is_none() || (same_block(current, block) && has_face(current, d)) {
        return false;
    }
    can_attach_to(r.get(p.relative(d)), d)
}

/// `MultifaceBlock.getStateForPlacement(current, level, pos, dir)`.
pub fn state_for_placement(block: u16, r: &mut Region, current: u16, p: BlockPos, d: Dir) -> Option<u16> {
    if !is_valid_for_placement(block, r, current, p, d) {
        return None;
    }
    let base = if same_block(current, block) {
        current
    } else if fluid(current).is_water_source() {
        with_prop(block, "waterlogged", "true")
    } else {
        block
    };
    Some(with_prop(base, d.name(), "true"))
}

/// The `MultifaceSpreader` of a spreadable block: the default one, or a sculk vein's.
#[derive(Clone, Copy, Debug)]
pub struct Spreader {
    /// The block's default state.
    pub block: u16,
    sculk: bool,
}

#[derive(Clone, Copy, Debug)]
struct SpreadPos {
    pos: BlockPos,
    face: Dir,
}

impl Spreader {
    pub fn of(block: u16) -> Self {
        Self { block, sculk: is_block(block, "minecraft:sculk_vein") }
    }

    /// `SpreadConfig.isOtherBlockValidAsSource`.
    fn other_valid_as_source(&self, s: u16) -> bool {
        self.sculk && !is_block(s, "minecraft:sculk_vein")
    }

    /// `stateCanBeReplaced(level, pos, spreadPos, face, state)`.
    fn can_replace(&self, r: &mut Region, from: BlockPos, to: BlockPos, face: Dir, s: u16) -> bool {
        if self.sculk {
            let n = r.get(to.relative(face));
            if ["minecraft:sculk", "minecraft:sculk_catalyst", "minecraft:moving_piston"].contains(&block_of(n).name) {
                return false;
            }
            if from.dist_manhattan(to) == 2 {
                let q = from.relative(face.opposite());
                if crate::block_facts::is_face_sturdy(r.get(q), face, crate::block_facts::Support::Full) {
                    return false;
                }
            }
            let f = fluid(s);
            if !f.is_empty() && f.kind != crate::block_facts::FluidKind::Water {
                return false;
            }
            if crate::vtags::is(s, "fire") {
                return false;
            }
            if kiln_data::block_props::replaceable(s) {
                return true;
            }
        }
        is_air(s) || same_block(s, self.block) || (is_block(s, "minecraft:water") && fluid(s).source)
    }

    /// `canSpreadInto`.
    fn can_spread_into(&self, r: &mut Region, from: BlockPos, sp: SpreadPos) -> bool {
        let s = r.get(sp.pos);
        self.can_replace(r, from, sp.pos, sp.face, s) && is_valid_for_placement(self.block, r, s, sp.pos, sp.face)
    }

    /// `getSpreadFromFaceTowardDirection` with `canSpreadInto`.
    fn spread_target(&self, r: &mut Region, s: u16, p: BlockPos, from: Dir, to: Dir) -> Option<SpreadPos> {
        let axis = |d: Dir| d as usize / 2;
        if axis(to) == axis(from) {
            return None;
        }
        if !self.other_valid_as_source(s) && (!has_face(s, from) || has_face(s, to)) {
            return None;
        }
        let candidates = [
            SpreadPos { pos: p, face: to },
            SpreadPos { pos: p.relative(to), face: from },
            SpreadPos { pos: p.relative(to).relative(from), face: to.opposite() },
        ];
        candidates.into_iter().find(|&sp| self.can_spread_into(r, p, sp))
    }

    /// `spreadToFace` (`SpreadConfig.placeBlock`).
    fn spread_to_face(&self, r: &mut Region, sp: SpreadPos, mark: bool) -> bool {
        let s = r.get(sp.pos);
        let Some(new) = state_for_placement(self.block, r, s, sp.pos, sp.face) else { return false };
        if mark {
            r.mark_post_processing(sp.pos);
        }
        r.set(sp.pos, new, 2)
    }

    /// `spreadFromFaceTowardRandomDirection`: whether it spread.
    pub fn spread_from_face_toward_random_direction(
        &self,
        r: &mut Region,
        s: u16,
        p: BlockPos,
        from: Dir,
        random: &mut WorldgenRandom,
        mark: bool,
    ) -> bool {
        for to in all_shuffled(random) {
            if let Some(sp) = self.spread_target(r, s, p, from, to)
                && self.spread_to_face(r, sp, mark)
            {
                return true;
            }
        }
        false
    }
}
