//! `BlockPredicate` as `match_block` uses it: the block and state parts are evaluated here, the
//! block entity parts (NBT, components) through [`crate::LootContext::block_entity_matches`].

use crate::json::Json;
use crate::parse::{IdSet, PResult, Parser, obj, opt, value};
use kiln_data::blocks_types::{BlockInfo, block_of};
use kiln_item::component::{DataComponentMatchers, NbtPredicate, StatePropertiesPredicate, ValueMatcher};
use kiln_item::registry;

/// The block entity half of a `BlockPredicate`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BlockEntityPredicate {
    pub nbt: Option<NbtPredicate>,
    pub components: DataComponentMatchers,
}

impl BlockEntityPredicate {
    /// `BlockPredicate.willMatchBlockEntity`.
    pub fn is_empty(&self) -> bool {
        self.nbt.is_none() && self.components.is_empty()
    }
}

/// `BlockPredicate` (`blocks`, `state`, `nbt`, `components`, `predicates`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BlockPredicate {
    pub blocks: Option<IdSet>,
    pub state: Option<StatePropertiesPredicate>,
    pub block_entity: BlockEntityPredicate,
}

impl BlockPredicate {
    pub fn parse(p: &Parser, j: &Json) -> PResult<BlockPredicate> {
        obj(j)?;
        let fields = j.to_value();
        let map = fields.as_map().map_err(|e| crate::parse::ParseError::new(e.0))?;
        Ok(BlockPredicate {
            blocks: opt(j, "blocks", |v| p.id_set(v, registry::BLOCK))?,
            state: opt(j, "state", |v| value(v, StatePropertiesPredicate::from_value))?,
            block_entity: BlockEntityPredicate {
                nbt: opt(j, "nbt", |v| value(v, NbtPredicate::from_value))?,
                components: DataComponentMatchers::from_fields(map).map_err(|e| crate::parse::ParseError::new(e.0))?,
            },
        })
    }

    /// `BlockPredicate.matchesState`.
    pub fn matches_state(&self, state: u16) -> bool {
        let block = block_of(state);
        if let Some(blocks) = &self.blocks {
            let Some(id) = registry::BLOCK.id(block.name) else { return false };
            if !blocks.contains(id) {
                return false;
            }
        }
        self.state.as_ref().is_none_or(|s| state_matches(s, block, state))
    }
}

/// `StatePropertiesPredicate.matches(BlockState)`.
pub fn state_matches(p: &StatePropertiesPredicate, block: &BlockInfo, state: u16) -> bool {
    p.0.iter().all(|m| {
        let Some(pi) = block.properties.iter().position(|pr| pr.name == m.name) else { return false };
        let prop = &block.properties[pi];
        let current = block.property_indices(state)[pi];
        let kind = PropertyKind::of(prop.values);
        match &m.value {
            ValueMatcher::Exact(v) => kind.parse(prop.values, v).is_some_and(|i| kind.cmp(prop.values, current, i).is_eq()),
            ValueMatcher::Range { min, max } => {
                if let Some(min) = min {
                    match kind.parse(prop.values, min) {
                        Some(i) if kind.cmp(prop.values, current, i).is_ge() => {}
                        _ => return false,
                    }
                }
                if let Some(max) = max {
                    match kind.parse(prop.values, max) {
                        Some(i) if kind.cmp(prop.values, current, i).is_le() => {}
                        _ => return false,
                    }
                }
                true
            }
        }
    })
}

/// How a property's values compare (`Comparable` of `IntegerProperty`, `BooleanProperty`,
/// `EnumProperty`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropertyKind {
    Int,
    Bool,
    Enum,
}

impl PropertyKind {
    fn of(values: &[&str]) -> PropertyKind {
        if values == ["true", "false"] {
            PropertyKind::Bool
        } else if values.iter().all(|v| v.parse::<i32>().is_ok()) {
            PropertyKind::Int
        } else {
            PropertyKind::Enum
        }
    }

    /// `Property.getValue(String)`: the value index, if the string names an allowed value.
    fn parse(self, values: &[&str], s: &str) -> Option<usize> {
        match self {
            PropertyKind::Int => {
                let n: i32 = s.parse().ok()?;
                values.iter().position(|v| v.parse::<i32>().ok() == Some(n))
            }
            _ => values.iter().position(|v| *v == s),
        }
    }

    fn cmp(self, values: &[&str], a: usize, b: usize) -> std::cmp::Ordering {
        match self {
            PropertyKind::Int => values[a].parse::<i32>().unwrap_or(0).cmp(&values[b].parse::<i32>().unwrap_or(0)),
            PropertyKind::Bool => (values[a] == "true").cmp(&(values[b] == "true")),
            PropertyKind::Enum => a.cmp(&b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_data::blocks_types::block_by_name;

    fn matcher(json: &str) -> StatePropertiesPredicate {
        StatePropertiesPredicate::from_value(&Json::parse(json).unwrap().to_value()).unwrap()
    }

    #[test]
    fn state_properties_compare_by_type() {
        let wheat = block_by_name("minecraft:wheat").unwrap();
        let age7 = wheat.with_property(wheat.default, "age", "7").unwrap();
        assert!(state_matches(&matcher(r#"{"age": "7"}"#), wheat, age7));
        assert!(!state_matches(&matcher(r#"{"age": "7"}"#), wheat, wheat.default));
        assert!(state_matches(&matcher(r#"{"age": {"min": "3"}}"#), wheat, age7));
        assert!(!state_matches(&matcher(r#"{"age": {"max": "6"}}"#), wheat, age7));
        assert!(!state_matches(&matcher(r#"{"age": "9"}"#), wheat, age7));
        assert!(!state_matches(&matcher(r#"{"nope": "1"}"#), wheat, age7));
        let slab = block_by_name("minecraft:oak_slab").unwrap();
        let wet = slab.with_property(slab.default, "waterlogged", "true").unwrap();
        assert!(state_matches(&matcher(r#"{"waterlogged": {"min": "true"}}"#), slab, wet));
        assert!(!state_matches(&matcher(r#"{"waterlogged": {"max": "false"}}"#), slab, wet));
    }
}
