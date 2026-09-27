//! Block states as worldgen data names them, resolved to global state ids (`kiln-data`).

use crate::Error;
use crate::function::qualify;
use crate::json::Json;
use kiln_data::blocks_types::{block_by_name, block_of};

pub use kiln_data::blocks::default_state as state;
pub use kiln_data::blocks_types::{has_fluid, is_air};

/// `BlockState.CODEC`: a block name (its default state), or `{"Name", "Properties"}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockSpec {
    pub name: String,
    pub properties: Vec<(String, String)>,
}

pub fn parse_block_state(json: &Json) -> Result<BlockSpec, Error> {
    if let Some(name) = json.as_str() {
        return Ok(BlockSpec { name: qualify(name), properties: Vec::new() });
    }
    let name = json.get("Name").and_then(Json::as_str).ok_or_else(|| Error::Invalid(format!("bad block state {json:?}")))?;
    let properties = match json.get("Properties") {
        None => Vec::new(),
        Some(Json::Object(fields)) => fields
            .iter()
            .map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())).ok_or_else(|| Error::Invalid("bad property".into())))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(Error::Invalid("bad Properties".into())),
    };
    Ok(BlockSpec { name: qualify(name), properties })
}

impl BlockSpec {
    /// The global state id.
    pub fn resolve(&self) -> Result<u16, Error> {
        let block = block_by_name(&self.name).ok_or_else(|| Error::Invalid(format!("unknown block {}", self.name)))?;
        let mut s = block.default;
        for (k, v) in &self.properties {
            s = block.with_property(s, k, v).ok_or_else(|| Error::Invalid(format!("bad property {k}={v} of {}", self.name)))?;
        }
        Ok(s)
    }
}

/// Whether `state` is a state of the block named `name`.
pub fn is_block(state: u16, name: &str) -> bool {
    block_of(state).name == name
}

/// Lava of any level (`state.is(Blocks.LAVA)`).
pub fn is_lava(state: u16) -> bool {
    is_block(state, "minecraft:lava")
}

/// Water of any level (`state.is(Blocks.WATER)`).
pub fn is_water(state: u16) -> bool {
    is_block(state, "minecraft:water")
}
