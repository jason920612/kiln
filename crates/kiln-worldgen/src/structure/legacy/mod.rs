//! Structures built from hard-coded pieces (`levelgen.structure.structures.*Pieces`), and the
//! `StructurePiecesBuilder` helpers they use.

pub mod buried_treasure;
pub mod mineshaft;
pub mod monument;
pub mod scattered;

use super::bbox::BoundingBox;
use super::piece::Piece;
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

/// The default state of a block by id.
pub fn st(name: &str) -> u16 {
    crate::blocks::block(name).unwrap_or_else(|_| panic!("unknown block {name}")).default
}

/// A state with properties changed.
pub fn with(s: u16, props: &[(&str, &str)]) -> u16 {
    props.iter().fold(s, |s, (k, v)| crate::blocks::with_prop(s, k, v))
}

/// `CompoundTag.putBoolean`.
pub fn bool_tag(b: bool) -> Tag {
    Tag::Byte(b as i8)
}

/// `StructurePiece.findCollisionPiece`: whether any box intersects `b`.
pub fn collides<'a>(mut boxes: impl Iterator<Item = &'a BoundingBox>, b: &BoundingBox) -> bool {
    boxes.any(|o| o.intersects(b))
}

/// `StructurePiecesBuilder.offsetPiecesVertically`.
pub fn offset_vertically(pieces: &mut [Box<dyn Piece>], dy: i32) {
    for p in pieces {
        p.shift(0, dy, 0);
    }
}

/// `StructurePiecesBuilder.moveBelowSeaLevel`: returns the offset applied.
pub fn move_below_sea_level(pieces: &mut [Box<dyn Piece>], sea_level: i32, min_y: i32, random: &mut WorldgenRandom, offset: i32) -> i32 {
    let max = sea_level - offset;
    let Some(b) = super::pieces_bbox(pieces) else { return 0 };
    let mut y = b.y_span() + min_y + 1;
    if y < max {
        y += random.next_int_bounded(max - y);
    }
    let dy = y - b.max_y;
    offset_vertically(pieces, dy);
    dy
}

/// `StructurePiecesBuilder.moveInsideHeights`.
pub fn move_inside_heights(pieces: &mut [Box<dyn Piece>], random: &mut WorldgenRandom, lo: i32, hi: i32) {
    let Some(b) = super::pieces_bbox(pieces) else { return };
    let room = hi - lo + 1 - b.y_span();
    let y = if room > 1 { lo + random.next_int_bounded(room) } else { lo };
    offset_vertically(pieces, y - b.min_y);
}

/// `SpawnerBlockEntity.setEntityId`: the spawner at `p` spawns `entity` (no randomness: its
/// spawn potentials are empty).
pub fn set_spawner_entity(r: &mut Region, p: BlockPos, entity: &str) {
    if let Some(Tag::Compound(fields)) = r.block_entity_mut(p) {
        fields.retain(|(k, _)| k != "SpawnData");
        let entity = Tag::Compound(vec![("id".into(), Tag::String(entity.into()))]);
        fields.push(("SpawnData".into(), Tag::Compound(vec![("entity".into(), entity)])));
    }
}
