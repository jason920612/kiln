//! Structure types (`StructureType`): each finds where a structure starts in a chunk and
//! builds its pieces. New types are added to [`parse`].

use super::{GenCtx, Stub};
use crate::Error;
use crate::json::Json;
use crate::sets::Loader;

/// A structure type's configuration and generation (`Structure.findGenerationPoint`).
pub trait Kind: Send + Sync {
    /// Where the structure would start in `ctx.chunk`, and how to build it; `None` if it does
    /// not generate there. The start position's biome is checked afterwards.
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>>;

    /// For an unimplemented type: its index in the gap counters.
    fn gap(&self) -> Option<usize> {
        None
    }

    /// `Structure.afterPlace`: runs after the start's pieces were placed in a chunk.
    fn after_place(
        &self,
        _cx: &super::piece::PlaceContext,
        _r: &mut crate::region::Region,
        _random: &mut crate::random::WorldgenRandom,
        _chunk_box: &super::bbox::BoundingBox,
        _chunk: (i32, i32),
        _start: &super::Start,
    ) {
    }
}

/// A structure type Kiln does not implement; generates nothing.
pub struct Unsupported(pub usize);

impl Kind for Unsupported {
    fn find<'k>(&'k self, _ctx: &mut GenCtx) -> Option<Stub<'k>> {
        None
    }

    fn gap(&self) -> Option<usize> {
        Some(self.0)
    }
}

/// The structure type named `ty` (without `minecraft:`) configured by `json`; `None` if Kiln
/// does not implement it.
pub fn parse(ty: &str, json: &Json, l: &Loader) -> Result<Option<Box<dyn Kind>>, Error> {
    Ok(match ty {
        "jigsaw" => Some(Box::new(super::jigsaw::parse(json, l)?)),
        _ => None,
    })
}
