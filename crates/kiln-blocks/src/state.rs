//! Block identity and property access over kiln-data's state tables.

use crate::pos::Direction;
use kiln_data::block_logic as logic;
use kiln_data::blocks::BLOCKS;
use kiln_data::blocks_types::BlockInfo;
use std::collections::HashMap;
use std::sync::OnceLock;

/// A block (index into `kiln_data::blocks::BLOCKS`), as opposed to one of its states.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u16);

impl std::fmt::Debug for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl BlockId {
    pub fn of(state: u16) -> Self {
        BlockId(logic::block_index(state) as u16)
    }

    pub fn by_name(name: &str) -> Option<Self> {
        static INDEX: OnceLock<HashMap<&'static str, u16>> = OnceLock::new();
        let index = INDEX.get_or_init(|| BLOCKS.iter().enumerate().map(|(i, b)| (b.name, i as u16)).collect());
        let name = name.strip_prefix("minecraft:").map_or_else(|| format!("minecraft:{name}"), |_| name.to_string());
        index.get(name.as_str()).map(|&i| BlockId(i))
    }

    pub fn info(self) -> &'static BlockInfo {
        &BLOCKS[self.0 as usize]
    }

    pub fn name(self) -> &'static str {
        self.info().name
    }

    pub fn default_state(self) -> u16 {
        self.info().default
    }
}

/// Whether `state` is a state of the block whose default state is `default`
/// (`BlockState.is(Block)`), e.g. `is(s, default_state::REDSTONE_WIRE)`.
pub fn is(state: u16, default: u16) -> bool {
    logic::block_index(state) == logic::block_index(default)
}

pub fn same_block(a: u16, b: u16) -> bool {
    logic::block_index(a) == logic::block_index(b)
}

/// Block info, property index and stride of property `name` in `state`'s block.
fn locate(state: u16, name: &str) -> Option<(&'static BlockInfo, usize, u16)> {
    let b = &BLOCKS[logic::block_index(state)];
    let mut stride = 1u16;
    for (i, p) in b.properties.iter().enumerate().rev() {
        if p.name == name {
            return Some((b, i, stride));
        }
        stride *= p.values.len() as u16;
    }
    None
}

pub fn has(state: u16, name: &str) -> bool {
    locate(state, name).is_some()
}

/// Index of the property's current value in its value list.
pub fn value_index(state: u16, name: &str) -> Option<usize> {
    let (b, i, stride) = locate(state, name)?;
    Some(((state - b.first) / stride) as usize % b.properties[i].values.len())
}

pub fn get(state: u16, name: &str) -> Option<&'static str> {
    let (b, i, stride) = locate(state, name)?;
    let p = &b.properties[i];
    Some(p.values[((state - b.first) / stride) as usize % p.values.len()])
}

/// A boolean property; false when the block does not have it.
pub fn get_bool(state: u16, name: &str) -> bool {
    get(state, name) == Some("true")
}

/// An integer property; 0 when the block does not have it.
pub fn get_int(state: u16, name: &str) -> i32 {
    get(state, name).and_then(|v| v.parse().ok()).unwrap_or(0)
}

pub fn get_dir(state: u16, name: &str) -> Option<Direction> {
    get(state, name).and_then(Direction::from_name)
}

/// `state` with property `name` set to `value`; unchanged if the block lacks the property
/// or the value.
pub fn set(state: u16, name: &str, value: &str) -> u16 {
    let Some((b, i, stride)) = locate(state, name) else { return state };
    let p = &b.properties[i];
    let Some(vi) = p.values.iter().position(|v| *v == value) else { return state };
    let cur = ((state - b.first) / stride) as usize % p.values.len();
    state - cur as u16 * stride + vi as u16 * stride
}

pub fn set_bool(state: u16, name: &str, value: bool) -> u16 {
    set(state, name, if value { "true" } else { "false" })
}

pub fn set_int(state: u16, name: &str, value: i32) -> u16 {
    let Some((b, i, stride)) = locate(state, name) else { return state };
    let p = &b.properties[i];
    let Some(vi) = p.values.iter().position(|v| v.parse::<i32>().ok() == Some(value)) else { return state };
    let cur = ((state - b.first) / stride) as usize % p.values.len();
    state - cur as u16 * stride + vi as u16 * stride
}

pub fn set_dir(state: u16, name: &str, dir: Direction) -> u16 {
    set(state, name, dir.name())
}

/// Copies every property the two blocks share from `from` onto `to`
/// (`BlockState.withPropertiesOf`).
pub fn with_properties_of(to: u16, from: u16) -> u16 {
    let fb = &BLOCKS[logic::block_index(from)];
    let mut s = to;
    for p in fb.properties {
        if let Some(v) = get(from, p.name) {
            s = set(s, p.name, v);
        }
    }
    s
}

/// Parses `minecraft:name[prop=value,...]` (the `/setblock` block state syntax).
pub fn parse_state(text: &str) -> Option<u16> {
    let (name, props) = match text.find('[') {
        Some(i) => (&text[..i], text[i + 1..].strip_suffix(']')?),
        None => (text, ""),
    };
    let mut s = BlockId::by_name(name)?.default_state();
    for kv in props.split(',').filter(|kv| !kv.trim().is_empty()) {
        let (k, v) = kv.split_once('=')?;
        let before = s;
        s = set(s, k.trim(), v.trim());
        if s == before && get(s, k.trim()) != Some(v.trim()) {
            return None;
        }
    }
    Some(s)
}

/// `minecraft:name[prop=value,...]` for a state (all properties, in definition order).
pub fn state_string(state: u16) -> String {
    let b = &BLOCKS[logic::block_index(state)];
    if b.properties.is_empty() {
        return b.name.to_string();
    }
    let props: Vec<String> = b.properties.iter().map(|p| format!("{}={}", p.name, get(state, p.name).unwrap())).collect();
    format!("{}[{}]", b.name, props.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_data::blocks::default_state as d;

    #[test]
    fn properties() {
        let s = parse_state("minecraft:repeater[delay=3,facing=east,locked=true]").unwrap();
        assert!(is(s, d::REPEATER));
        assert_eq!(get_int(s, "delay"), 3);
        assert_eq!(get_dir(s, "facing"), Some(Direction::East));
        assert!(get_bool(s, "locked") && !get_bool(s, "powered"));
        let s2 = set_int(s, "delay", 1);
        assert_eq!(get_int(s2, "delay"), 1);
        assert_eq!(get_dir(s2, "facing"), Some(Direction::East));
        assert_eq!(state_string(s), "minecraft:repeater[delay=3,facing=east,locked=true,powered=false]");
        assert_eq!(parse_state(&state_string(s2)), Some(s2));
        assert_eq!(parse_state("stone"), Some(d::STONE));
        assert_eq!(parse_state("minecraft:repeater[delay=9]"), None);
        let wire = set_int(d::REDSTONE_WIRE, "power", 15);
        assert_eq!(get_int(wire, "power"), 15);
        assert_eq!(BlockId::of(wire).name(), "minecraft:redstone_wire");
    }
}
