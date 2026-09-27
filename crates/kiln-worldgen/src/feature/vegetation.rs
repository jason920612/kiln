//! Plant and cave features: block columns, vegetation patches, vines, multiface growth, root systems, dripstone, sculk, block piles, bamboo, huge mushrooms and fungi, coral.

use crate::Error;
use crate::feature::Features;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;

/// The feature types of this family.
#[derive(Debug)]
pub enum Kind {}

/// Parses a feature of this family (`ty` without the `minecraft:` prefix); `None` if the
/// type is not one of them.
pub fn parse(_ty: &str, _json: &Json, _f: &mut Features, _l: &Loader) -> Option<Result<Kind, Error>> {
    None
}

impl Kind {
    pub fn place(&self, _f: &Features, _r: &mut Region, _random: &mut WorldgenRandom, _p: BlockPos) -> bool {
        match *self {}
    }

    pub fn type_name(&self) -> &'static str {
        match *self {}
    }

    /// Placed features this feature places (for [`Features::is_supported`]).
    pub fn nested(&self) -> Vec<usize> {
        match *self {}
    }
}
