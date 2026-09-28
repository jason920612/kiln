//! State and biome id tables. Native chunk records store numeric block state and biome ids;
//! each record names the table its ids refer to by a fingerprint, and the tables live in
//! `registries/<fingerprint>.bin` next to the cell files. A record written by a build whose
//! ids differ (a new game version reorders block states) is remapped by name on load.

use crate::anvil::DATA_VERSION;
use kiln_data::blocks::STATE_COUNT;
use kiln_data::blocks_types::{block_by_name, block_of};
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::OnceLock;

pub struct Registry {
    pub data_version: i32,
    pub states: Vec<String>,
    pub biomes: Vec<String>,
    pub fingerprint: u32,
}

/// `minecraft:oak_fence[east=false,north=true,...]`, properties in declaration order.
pub fn state_string(state: u16) -> String {
    let block = block_of(state);
    if block.properties.is_empty() {
        return block.name.to_owned();
    }
    let props: Vec<String> =
        block.properties.iter().zip(block.property_indices(state)).map(|(p, i)| format!("{}={}", p.name, p.values[i])).collect();
    format!("{}[{}]", block.name, props.join(","))
}

fn parse_state(s: &str) -> Option<u16> {
    let (name, props) = match s.split_once('[') {
        Some((n, rest)) => (n, rest.strip_suffix(']')?),
        None => (s, ""),
    };
    let block = block_by_name(name)?;
    let mut state = block.default;
    for kv in props.split(',').filter(|p| !p.is_empty()) {
        let (k, v) = kv.split_once('=')?;
        state = block.with_property(state, k, v)?;
    }
    Some(state)
}

impl Registry {
    fn new(data_version: i32, states: Vec<String>, biomes: Vec<String>) -> Registry {
        let mut h = crc32fast::Hasher::new();
        h.update(&data_version.to_le_bytes());
        for s in states.iter().chain(&biomes) {
            h.update(s.as_bytes());
            h.update(&[0]);
        }
        // 0 means "no table".
        let fingerprint = h.finalize().max(1);
        Registry { data_version, states, biomes, fingerprint }
    }

    fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"KREG");
        b.extend_from_slice(&self.data_version.to_le_bytes());
        for list in [&self.states, &self.biomes] {
            b.extend_from_slice(&(list.len() as u32).to_le_bytes());
            for s in list.iter() {
                b.extend_from_slice(&(s.len() as u16).to_le_bytes());
                b.extend_from_slice(s.as_bytes());
            }
        }
        zstd::bulk::compress(&b, 3).expect("in-memory compression")
    }

    fn decode(data: &[u8]) -> Option<Registry> {
        let b = zstd::bulk::decompress(data, 16 << 20).ok()?;
        let mut r = &b[..];
        let mut take = |n: usize| -> Option<&[u8]> {
            let (h, t) = r.split_at_checked(n)?;
            r = t;
            Some(h)
        };
        if take(4)? != b"KREG" {
            return None;
        }
        let data_version = i32::from_le_bytes(take(4)?.try_into().ok()?);
        let mut lists = [Vec::new(), Vec::new()];
        for list in &mut lists {
            let n = u32::from_le_bytes(take(4)?.try_into().ok()?);
            for _ in 0..n {
                let len = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
                list.push(String::from_utf8(take(len)?.to_vec()).ok()?);
            }
        }
        let [states, biomes] = lists;
        Some(Registry::new(data_version, states, biomes))
    }

    /// This build's table.
    pub fn current() -> &'static Registry {
        static CURRENT: OnceLock<Registry> = OnceLock::new();
        CURRENT.get_or_init(|| {
            let states = (0..STATE_COUNT as u16).map(state_string).collect();
            let biomes = crate::anvil::biome_names().iter().map(|s| (*s).to_owned()).collect();
            Registry::new(DATA_VERSION as i32, states, biomes)
        })
    }

    /// Writes this build's table into `dir/registries` unless it is there.
    pub fn save_current(dir: &Path) -> io::Result<()> {
        let reg = Registry::current();
        let path = dir.join("registries").join(format!("{:08x}.bin", reg.fingerprint));
        if path.exists() {
            return Ok(());
        }
        std::fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_extension("bin.tmp");
        std::fs::write(&tmp, reg.encode())?;
        std::fs::rename(tmp, path)
    }

    /// The table saved under `fingerprint`.
    pub fn load(dir: &Path, fingerprint: u32) -> Option<Registry> {
        let data = std::fs::read(dir.join("registries").join(format!("{fingerprint:08x}.bin"))).ok()?;
        Registry::decode(&data).filter(|r| r.fingerprint == fingerprint)
    }
}

/// Maps another table's ids to this build's.
pub struct Remap {
    pub states: Vec<u16>,
    pub biomes: Vec<u16>,
}

impl Remap {
    pub fn to_current(from: &Registry) -> Remap {
        let air = kiln_data::blocks::default_state::AIR;
        let mut unknown = 0;
        let states = from
            .states
            .iter()
            .map(|s| {
                parse_state(s).unwrap_or_else(|| {
                    unknown += 1;
                    air
                })
            })
            .collect();
        if unknown > 0 {
            tracing::warn!("{unknown} block states of a stored id table are unknown to this build; they load as air");
        }
        let current: HashMap<&str, u16> =
            Registry::current().biomes.iter().enumerate().map(|(i, n)| (n.as_str(), i as u16)).collect();
        let plains = current.get("minecraft:plains").copied().unwrap_or(0);
        let biomes = from.biomes.iter().map(|n| current.get(n.as_str()).copied().unwrap_or(plains)).collect();
        Remap { states, biomes }
    }

    pub fn state(&self, id: u16) -> u16 {
        self.states.get(id as usize).copied().unwrap_or(kiln_data::blocks::default_state::AIR)
    }

    pub fn biome(&self, id: u16) -> u16 {
        self.biomes.get(id as usize).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_strings_parse_back() {
        for s in (0..STATE_COUNT as u16).step_by(97) {
            assert_eq!(parse_state(&state_string(s)), Some(s), "{}", state_string(s));
        }
    }

    #[test]
    fn tables_round_trip_and_remap() {
        let cur = Registry::current();
        let back = Registry::decode(&cur.encode()).unwrap();
        assert_eq!(back.fingerprint, cur.fingerprint);
        // A table with two states swapped and a biome gone maps back by name.
        let mut states = cur.states.clone();
        states.swap(1, 2);
        let mut biomes = cur.biomes.clone();
        biomes.insert(0, "example:gone".into());
        let other = Registry::new(DATA_VERSION as i32 - 1, states, biomes);
        let r = Remap::to_current(&other);
        assert_eq!((r.state(1), r.state(2), r.state(3)), (2, 1, 3));
        assert_eq!(r.biome(1), 0);
        assert_eq!(cur.biomes[r.biome(0) as usize], "minecraft:plains");
    }
}
