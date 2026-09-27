//! Block states as worldgen data names them, resolved to global state ids (`kiln-data`).

use crate::Error;
use crate::function::qualify;
use crate::json::Json;
use kiln_data::blocks_types::{block_by_name, block_of};

pub use kiln_data::blocks::default_state as state;
pub use kiln_data::blocks_types::{has_fluid, is_air};

/// `BlockState.CODEC`: a block name (its default state), or `{"id", "properties"}` (older
/// data: `{"Name", "Properties"}`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockSpec {
    pub name: String,
    pub properties: Vec<(String, String)>,
}

pub fn parse_block_state(json: &Json) -> Result<BlockSpec, Error> {
    if let Some(name) = json.as_str() {
        return Ok(BlockSpec { name: qualify(name), properties: Vec::new() });
    }
    let name = json
        .get("id")
        .or_else(|| json.get("Name"))
        .and_then(Json::as_str)
        .ok_or_else(|| Error::Invalid(format!("bad block state {json:?}")))?;
    let properties = match json.get("properties").or_else(|| json.get("Properties")) {
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

/// Parses and resolves a block state.
pub fn block_state(json: &Json) -> Result<u16, Error> {
    parse_block_state(json)?.resolve()
}

/// The first state of a block by name (states of one block are contiguous).
pub fn block(name: &str) -> Result<&'static kiln_data::blocks_types::BlockInfo, Error> {
    block_by_name(&qualify(name)).ok_or_else(|| Error::Invalid(format!("unknown block {name}")))
}

/// Whether two states belong to the same block.
#[inline]
pub fn same_block(a: u16, b: u16) -> bool {
    block_of(a).first == block_of(b).first
}

/// A property value of a state (`None` if the block lacks the property).
pub fn prop(state: u16, name: &str) -> Option<&'static str> {
    block_of(state).property(state, name)
}

/// `state` with a property changed; unchanged if the block lacks the property or value.
pub fn with_prop(state: u16, name: &str, value: &str) -> u16 {
    block_of(state).with_property(state, name, value).unwrap_or(state)
}

/// Whether the block has a property.
pub fn has_prop(state: u16, name: &str) -> bool {
    block_of(state).properties.iter().any(|p| p.name == name)
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
