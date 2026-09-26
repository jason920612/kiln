//! Registry entries by network id and by name, from kiln-data's generated tables. Synchronized
//! (data-driven) registries use the vanilla datapack's ids, as sent to clients.

use crate::ident::Identifier;
use crate::value::{DataError, DataResult, Value};
use kiln_proto::{DecodeError, Reader, WriteExt};
use std::collections::HashMap;
use std::sync::OnceLock;

/// A registry key such as `minecraft:item`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Registry(pub &'static str);

pub const ITEM: Registry = Registry("minecraft:item");
pub const BLOCK: Registry = Registry("minecraft:block");
pub const ENTITY_TYPE: Registry = Registry("minecraft:entity_type");
pub const BLOCK_ENTITY_TYPE: Registry = Registry("minecraft:block_entity_type");
pub const DATA_COMPONENT_TYPE: Registry = Registry("minecraft:data_component_type");
pub const SOUND_EVENT: Registry = Registry("minecraft:sound_event");
pub const MOB_EFFECT: Registry = Registry("minecraft:mob_effect");
pub const POTION: Registry = Registry("minecraft:potion");
pub const ATTRIBUTE: Registry = Registry("minecraft:attribute");
pub const PARTICLE_TYPE: Registry = Registry("minecraft:particle_type");
pub const VILLAGER_TYPE: Registry = Registry("minecraft:villager_type");
pub const MAP_DECORATION_TYPE: Registry = Registry("minecraft:map_decoration_type");
pub const CONSUME_EFFECT_TYPE: Registry = Registry("minecraft:consume_effect_type");
pub const ENCHANTMENT: Registry = Registry("minecraft:enchantment");
pub const DAMAGE_TYPE: Registry = Registry("minecraft:damage_type");
pub const TRIM_MATERIAL: Registry = Registry("minecraft:trim_material");
pub const TRIM_PATTERN: Registry = Registry("minecraft:trim_pattern");
pub const BANNER_PATTERN: Registry = Registry("minecraft:banner_pattern");
pub const JUKEBOX_SONG: Registry = Registry("minecraft:jukebox_song");
pub const INSTRUMENT: Registry = Registry("minecraft:instrument");
pub const PAINTING_VARIANT: Registry = Registry("minecraft:painting_variant");
pub const DECORATED_POT_PATTERN: Registry = Registry("minecraft:decorated_pot_pattern");
pub const BLOCK_TRANSFORMER: Registry = Registry("minecraft:block_transformer");
pub const DIALOG: Registry = Registry("minecraft:dialog");

impl Registry {
    /// Entries indexed by network id.
    pub fn entries(self) -> &'static [&'static str] {
        kiln_data::builtin_entries(self.0)
            .or_else(|| kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == self.0).map(|(_, e)| *e))
            .unwrap_or_else(|| panic!("unknown registry {}", self.0))
    }

    pub fn name(self, id: i32) -> Option<&'static str> {
        usize::try_from(id).ok().and_then(|i| self.entries().get(i).copied())
    }

    /// Network id of `name` (a missing namespace means `minecraft`).
    pub fn id(self, name: &str) -> Option<i32> {
        type Index = HashMap<&'static str, HashMap<&'static str, i32>>;
        static INDEX: OnceLock<Index> = OnceLock::new();
        let all = INDEX.get_or_init(|| {
            let regs = kiln_data::registries::BUILTIN.iter().chain(kiln_data::registries::SYNCHRONIZED);
            regs.map(|(r, entries)| (*r, entries.iter().enumerate().map(|(i, e)| (*e, i as i32)).collect())).collect()
        });
        let index = all.get(self.0)?;
        match index.get(name) {
            Some(&id) => Some(id),
            None => index.get(Identifier::parse(name)?.as_str()).copied(),
        }
    }

    pub fn len(self) -> usize {
        self.entries().len()
    }

    pub fn is_empty(self) -> bool {
        self.entries().is_empty()
    }

    /// `ByteBufCodecs.registry` / `holderRegistry`: a VarInt network id.
    pub fn read_id(self, r: &mut Reader<'_>) -> Result<i32, DecodeError> {
        let id = r.varint()?;
        if id < 0 || id as usize >= self.len() {
            return Err(DecodeError::Invalid("registry id out of range"));
        }
        Ok(id)
    }

    pub fn write_id(self, id: i32, out: &mut bytes::BytesMut) {
        out.put_varint(id);
    }

    /// The entry's name, as `Registry.byNameCodec()` and registry-reference holders encode it.
    pub fn id_to_value(self, id: i32) -> Value {
        Value::str(self.name(id).unwrap_or("minecraft:unknown"))
    }

    pub fn id_from_value(self, v: &Value) -> DataResult<i32> {
        let name = v.as_str()?;
        self.id(name).ok_or_else(|| DataError(format!("unknown {} entry {name:?}", self.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_both_ways() {
        let stick = ITEM.id("stick").unwrap();
        assert_eq!(ITEM.name(stick), Some("minecraft:stick"));
        assert_eq!(ITEM.id("minecraft:stick"), Some(stick));
        assert_eq!(ENCHANTMENT.id("minecraft:aqua_affinity"), Some(0));
        assert_eq!(ITEM.id("minecraft:nope"), None);
    }
}
