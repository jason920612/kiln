//! Sets of blocks, fluids and biomes as worldgen data names them (`HolderSet`s: an id, a list
//! of ids or a `#tag`), resolved against the loaded datapack.

use crate::Error;
use crate::block_facts::{Fluid, FluidKind};
use crate::datapack::Datapack;
use crate::function::qualify;
use crate::json::Json;
use kiln_data::blocks_types::block_by_name;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// A set of block states (a bit per state id).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockSet(Box<[u64]>);

impl BlockSet {
    pub fn empty() -> Self {
        BlockSet(vec![0u64; (kiln_data::blocks::STATE_COUNT as usize).div_ceil(64)].into_boxed_slice())
    }

    #[inline]
    pub fn contains(&self, state: u16) -> bool {
        self.0[state as usize >> 6] & (1 << (state & 63)) != 0
    }

    pub fn insert(&mut self, state: u16) {
        self.0[state as usize >> 6] |= 1 << (state & 63);
    }

    /// Adds every state of the named block.
    pub fn insert_block(&mut self, name: &str) -> Result<(), Error> {
        let b = block_by_name(&qualify(name)).ok_or_else(|| Error::Invalid(format!("unknown block {name}")))?;
        for s in b.first..=b.last {
            self.insert(s);
        }
        Ok(())
    }
}

/// A set of fluid types (`HolderSet<Fluid>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FluidSet(u8);

impl FluidSet {
    #[inline]
    pub fn contains(self, f: Fluid) -> bool {
        self.0 & (1 << f.kind as u8) != 0
    }

    pub fn contains_kind(self, k: FluidKind) -> bool {
        self.0 & (1 << k as u8) != 0
    }
}

/// A set of biome indices (`HolderSet<Biome>`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BiomeSet(Vec<bool>);

impl BiomeSet {
    #[inline]
    pub fn contains(&self, biome: u16) -> bool {
        self.0.get(biome as usize).copied().unwrap_or(false)
    }
}

/// Resolves names while worldgen data is parsed; caches tag lookups.
pub struct Loader<'a> {
    pub pack: &'a Datapack,
    /// Biome ids in index order (`Generator::biomes`).
    pub biome_names: Vec<String>,
    block_tags: RefCell<HashMap<String, Arc<BlockSet>>>,
}

impl<'a> Loader<'a> {
    pub fn new(pack: &'a Datapack, biome_names: Vec<String>) -> Self {
        Self { pack, biome_names, block_tags: RefCell::new(HashMap::new()) }
    }

    /// A block tag (without `#`) as a state set.
    pub fn block_tag(&self, tag: &str) -> Result<Arc<BlockSet>, Error> {
        let tag = qualify(tag.trim_start_matches('#'));
        if let Some(s) = self.block_tags.borrow().get(&tag) {
            return Ok(s.clone());
        }
        let mut set = BlockSet::empty();
        for name in self.pack.block_tag(&tag)? {
            // Tags may name blocks of other versions or optional entries; skip unknown ones.
            let _ = set.insert_block(&name);
        }
        let set = Arc::new(set);
        self.block_tags.borrow_mut().insert(tag, set.clone());
        Ok(set)
    }

    /// `RegistryCodecs.homogeneousList(BLOCK)`: `"id"`, `["id", ...]` or `"#tag"`.
    pub fn blocks(&self, json: &Json) -> Result<Arc<BlockSet>, Error> {
        match json {
            Json::String(s) if s.starts_with('#') => self.block_tag(s),
            Json::String(s) => {
                let mut set = BlockSet::empty();
                set.insert_block(s)?;
                Ok(Arc::new(set))
            }
            Json::Array(items) => {
                let mut set = BlockSet::empty();
                for i in items {
                    let name = i.as_str().ok_or_else(|| Error::Invalid(format!("bad block list {json:?}")))?;
                    set.insert_block(name)?;
                }
                Ok(Arc::new(set))
            }
            _ => Err(Error::Invalid(format!("bad block set {json:?}"))),
        }
    }

    /// `HolderSet<Fluid>`.
    pub fn fluids(&self, json: &Json) -> Result<FluidSet, Error> {
        let names: Vec<String> = match json {
            Json::String(s) if s.starts_with('#') => self.pack.tag("fluid", s)?,
            Json::String(s) => vec![qualify(s)],
            Json::Array(items) => items.iter().filter_map(|i| i.as_str().map(qualify)).collect(),
            _ => return Err(Error::Invalid(format!("bad fluid set {json:?}"))),
        };
        let mut bits = 0u8;
        for n in names {
            let k = match n.as_str() {
                "minecraft:empty" => FluidKind::Empty,
                "minecraft:flowing_water" => FluidKind::FlowingWater,
                "minecraft:water" => FluidKind::Water,
                "minecraft:flowing_lava" => FluidKind::FlowingLava,
                "minecraft:lava" => FluidKind::Lava,
                _ => return Err(Error::Invalid(format!("unknown fluid {n}"))),
            };
            bits |= 1 << k as u8;
        }
        Ok(FluidSet(bits))
    }

    /// Index of a biome by id.
    pub fn biome(&self, name: &str) -> Result<u16, Error> {
        let name = qualify(name);
        self.biome_names
            .iter()
            .position(|b| *b == name)
            .map(|i| i as u16)
            .ok_or_else(|| Error::Invalid(format!("unknown biome {name}")))
    }

    /// `HolderSet<Biome>`.
    pub fn biomes(&self, json: &Json) -> Result<BiomeSet, Error> {
        let names: Vec<String> = match json {
            Json::String(s) if s.starts_with('#') => self.pack.tag("worldgen/biome", s)?,
            Json::String(s) => vec![qualify(s)],
            Json::Array(items) => items.iter().filter_map(|i| i.as_str().map(qualify)).collect(),
            _ => return Err(Error::Invalid(format!("bad biome set {json:?}"))),
        };
        let mut set = vec![false; self.biome_names.len()];
        for n in names {
            if let Ok(i) = self.biome(&n) {
                set[i as usize] = true;
            }
        }
        Ok(BiomeSet(set))
    }
}
