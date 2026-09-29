//! Points of interest (`PoiManager`, `PoiSection`, `PoiRecord`, `PoiTypes`): the beds, job
//! sites, bells, bee homes, portals, lodestones and lightning rods of a chunk, kept per section
//! with their free tickets, as vanilla saves them in `poi/r.<x>.<z>.mca`. A chunk's points of
//! interest live with the chunk (so they follow it between regions) and follow its block
//! changes ([`crate::chunk::Chunk::set`]).

use kiln_proto::nbt::Tag;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// A `minecraft:point_of_interest_type` (`PoiType`): its tickets and valid range.
#[derive(Debug, Clone, Copy)]
pub struct PoiType {
    pub name: &'static str,
    pub max_tickets: i32,
    pub valid_range: i32,
}

const fn t(name: &'static str, max_tickets: i32, valid_range: i32) -> PoiType {
    PoiType { name, max_tickets, valid_range }
}

/// `PoiTypes.bootstrap`, in registry order.
pub const TYPES: [PoiType; 21] = [
    t("minecraft:armorer", 1, 1),
    t("minecraft:butcher", 1, 1),
    t("minecraft:cartographer", 1, 1),
    t("minecraft:cleric", 1, 1),
    t("minecraft:farmer", 1, 1),
    t("minecraft:fisherman", 1, 1),
    t("minecraft:fletcher", 1, 1),
    t("minecraft:leatherworker", 1, 1),
    t("minecraft:librarian", 1, 1),
    t("minecraft:mason", 1, 1),
    t("minecraft:shepherd", 1, 1),
    t("minecraft:toolsmith", 1, 1),
    t("minecraft:weaponsmith", 1, 1),
    t("minecraft:home", 1, 1),
    t("minecraft:meeting", 32, 6),
    t("minecraft:beehive", 0, 1),
    t("minecraft:bee_nest", 0, 1),
    t("minecraft:nether_portal", 0, 1),
    t("minecraft:lodestone", 0, 1),
    t("minecraft:test_instance", 0, 1),
    t("minecraft:lightning_rod", 0, 1),
];

/// The block (or blocks) of each type.
fn type_of_block(name: &str, state: u16) -> Option<&'static str> {
    let path = name.strip_prefix("minecraft:")?;
    Some(match path {
        "blast_furnace" => "minecraft:armorer",
        "smoker" => "minecraft:butcher",
        "cartography_table" => "minecraft:cartographer",
        "brewing_stand" => "minecraft:cleric",
        "composter" => "minecraft:farmer",
        "barrel" => "minecraft:fisherman",
        "fletching_table" => "minecraft:fletcher",
        "cauldron" | "lava_cauldron" | "water_cauldron" | "powder_snow_cauldron" => "minecraft:leatherworker",
        "lectern" => "minecraft:librarian",
        "stonecutter" => "minecraft:mason",
        "loom" => "minecraft:shepherd",
        "smithing_table" => "minecraft:toolsmith",
        "grindstone" => "minecraft:weaponsmith",
        "bell" => "minecraft:meeting",
        "beehive" => "minecraft:beehive",
        "bee_nest" => "minecraft:bee_nest",
        "nether_portal" => "minecraft:nether_portal",
        "lodestone" => "minecraft:lodestone",
        "test_instance_block" => "minecraft:test_instance",
        p if p.ends_with("lightning_rod") => "minecraft:lightning_rod",
        // `Blocks.BED`: the head half of each colored bed.
        p if p.ends_with("_bed") => {
            let info = kiln_data::blocks_types::block_of(state);
            if info.property(state, "part") != Some("head") {
                return None;
            }
            "minecraft:home"
        }
        _ => return None,
    })
}

/// `PoiTypes.forState`: the type index of a block state (in [`TYPES`]).
pub fn type_of(state: u16) -> Option<u8> {
    static TABLE: OnceLock<Vec<u8>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut out = Vec::new();
        for b in kiln_data::blocks::BLOCKS {
            for s in b.first..=b.last {
                let i = type_of_block(b.name, s).and_then(|n| TYPES.iter().position(|t| t.name == n)).map_or(0, |i| i as u8 + 1);
                if out.len() <= s as usize {
                    out.resize(s as usize + 1, 0);
                }
                out[s as usize] = i;
            }
        }
        out
    });
    table.get(state as usize).copied().filter(|&i| i > 0).map(|i| i - 1)
}

/// `PoiTypes.hasPoi`.
pub fn has_poi(state: u16) -> bool {
    type_of(state).is_some()
}

/// The type index of a `minecraft:point_of_interest_type` name.
pub fn type_index(name: &str) -> Option<u8> {
    TYPES.iter().position(|t| t.name == name).map(|i| i as u8)
}

/// `#minecraft:village`: the job sites, homes and meeting points.
pub fn is_village(kind: u8) -> bool {
    kind <= 14
}

/// `#minecraft:acquirable_job_site`.
pub fn is_job_site(kind: u8) -> bool {
    kind <= 12
}

/// `PoiRecord`.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub pos: [i32; 3],
    pub kind: u8,
    pub free_tickets: i32,
}

impl Record {
    pub fn new(pos: [i32; 3], kind: u8) -> Record {
        Record { pos, kind, free_tickets: TYPES[kind as usize].max_tickets }
    }

    /// `hasSpace`.
    pub fn has_space(&self) -> bool {
        self.free_tickets > 0
    }

    /// `isOccupied`.
    pub fn is_occupied(&self) -> bool {
        self.free_tickets != TYPES[self.kind as usize].max_tickets
    }
}

/// `PoiManager.Occupancy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occupancy {
    HasSpace,
    IsOccupied,
    Any,
}

impl Occupancy {
    pub fn test(self, r: &Record) -> bool {
        match self {
            Occupancy::HasSpace => r.has_space(),
            Occupancy::IsOccupied => r.is_occupied(),
            Occupancy::Any => true,
        }
    }
}

/// `SectionPos.sectionRelativePos`.
fn rel(pos: [i32; 3]) -> u16 {
    (((pos[0] & 15) << 8) | ((pos[2] & 15) << 4) | (pos[1] & 15)) as u16
}

/// `PoiSection`: records by section-relative position.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Section {
    /// `isValid`: false for data saved before the blocks were scanned (refreshed on load).
    pub valid: bool,
    pub records: BTreeMap<u16, Record>,
}

impl Section {
    /// Records of the section passing `kinds` and `occupancy`.
    pub fn records<'a>(&'a self, kinds: &'a dyn Fn(u8) -> bool, occupancy: Occupancy) -> impl Iterator<Item = &'a Record> + 'a {
        self.records.values().filter(move |r| kinds(r.kind) && occupancy.test(r))
    }

    /// Whether an occupied village point of interest is in the section (`isVillageCenter`).
    pub fn is_village_center(&self) -> bool {
        self.records.values().any(|r| is_village(r.kind) && r.is_occupied())
    }
}

/// A chunk's points of interest by section y.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChunkPois {
    pub sections: BTreeMap<i32, Section>,
    /// Changed since it was loaded or saved.
    pub dirty: bool,
    /// The chunk's position (block changes come in chunk-relative coordinates).
    pub cx: i32,
    pub cz: i32,
}

impl ChunkPois {
    /// `PoiManager.add`: a new record where there was none.
    pub fn add(&mut self, pos: [i32; 3], kind: u8) {
        let s = self.sections.entry(pos[1] >> 4).or_insert_with(|| Section { valid: true, records: BTreeMap::new() });
        let key = rel(pos);
        if s.records.get(&key).is_some_and(|r| r.kind == kind) {
            return;
        }
        s.records.insert(key, Record::new(pos, kind));
        self.dirty = true;
    }

    /// `PoiManager.remove`.
    pub fn remove(&mut self, pos: [i32; 3]) {
        if let Some(s) = self.sections.get_mut(&(pos[1] >> 4))
            && s.records.remove(&rel(pos)).is_some()
        {
            self.dirty = true;
        }
    }

    pub fn get(&self, pos: [i32; 3]) -> Option<&Record> {
        self.sections.get(&(pos[1] >> 4))?.records.get(&rel(pos))
    }

    pub fn get_mut(&mut self, pos: [i32; 3]) -> Option<&mut Record> {
        self.sections.get_mut(&(pos[1] >> 4))?.records.get_mut(&rel(pos))
    }

    /// `PoiRecord.acquireTicket`.
    pub fn acquire(&mut self, pos: [i32; 3]) -> bool {
        let Some(r) = self.get_mut(pos) else { return false };
        if r.free_tickets <= 0 {
            return false;
        }
        r.free_tickets -= 1;
        self.dirty = true;
        true
    }

    /// `PoiManager.release` (`PoiRecord.releaseTicket`).
    pub fn release(&mut self, pos: [i32; 3]) -> bool {
        let Some(r) = self.get_mut(pos) else { return false };
        if r.free_tickets >= TYPES[r.kind as usize].max_tickets {
            return false;
        }
        r.free_tickets += 1;
        self.dirty = true;
        true
    }

    /// Whether section `y` holds an occupied village point of interest.
    pub fn is_village_center(&self, y: i32) -> bool {
        self.sections.get(&y).is_some_and(Section::is_village_center)
    }

    /// The saved form (`SectionStorage.writeChunk`): `Sections` by section y, and the data
    /// version.
    pub fn to_nbt(&self, data_version: i32) -> Tag {
        let sections = self
            .sections
            .iter()
            .map(|(y, s)| {
                let records = s
                    .records
                    .values()
                    .map(|r| {
                        Tag::Compound(vec![
                            ("pos".into(), Tag::IntArray(r.pos.to_vec())),
                            ("type".into(), Tag::String(TYPES[r.kind as usize].name.into())),
                            ("free_tickets".into(), Tag::Int(r.free_tickets)),
                        ])
                    })
                    .collect();
                (y.to_string(), Tag::Compound(vec![("Valid".into(), Tag::Byte(s.valid as i8)), ("Records".into(), Tag::List(records))]))
            })
            .collect();
        Tag::Compound(vec![("Sections".into(), Tag::Compound(sections)), ("DataVersion".into(), Tag::Int(data_version))])
    }

    /// Reads saved points of interest (unknown types and misplaced records are dropped).
    pub fn from_nbt(tag: &Tag) -> ChunkPois {
        let mut out = ChunkPois::default();
        let Some(Tag::Compound(sections)) = tag.get("Sections") else { return out };
        for (k, v) in sections {
            let Ok(y) = k.parse::<i32>() else { continue };
            let valid = v.get("Valid").and_then(Tag::as_i64).unwrap_or(0) != 0;
            let mut s = Section { valid, records: BTreeMap::new() };
            if let Some(list) = v.get("Records").and_then(Tag::as_list) {
                for r in list {
                    let r = r.unwrap_list_element();
                    let pos = match r.get("pos") {
                        Some(Tag::IntArray(p)) if p.len() == 3 => [p[0], p[1], p[2]],
                        _ => continue,
                    };
                    let Some(kind) = r.get("type").and_then(Tag::as_str).and_then(type_index) else { continue };
                    if pos[1] >> 4 != y {
                        continue;
                    }
                    let free = r.get("free_tickets").and_then(Tag::as_i64).unwrap_or(0) as i32;
                    s.records.insert(rel(pos), Record { pos, kind, free_tickets: free });
                }
            }
            out.sections.insert(y, s);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_of_blocks() {
        let bell = kiln_data::blocks_types::block_by_name("minecraft:bell").unwrap().default;
        assert_eq!(type_of(bell).map(|i| TYPES[i as usize].name), Some("minecraft:meeting"));
        let bed = kiln_data::blocks_types::block_by_name("minecraft:red_bed").unwrap();
        let heads = (bed.first..=bed.last).filter(|&s| type_of(s).is_some()).count();
        assert_eq!(heads * 2, (bed.last - bed.first + 1) as usize);
        let stone = kiln_data::blocks_types::block_by_name("minecraft:stone").unwrap().default;
        assert_eq!(type_of(stone), None);
        let rod = kiln_data::blocks_types::block_by_name("minecraft:waxed_oxidized_lightning_rod").unwrap().default;
        assert_eq!(type_of(rod).map(|i| TYPES[i as usize].name), Some("minecraft:lightning_rod"));
        assert!(is_village(type_index("minecraft:meeting").unwrap()));
        assert!(!is_village(type_index("minecraft:beehive").unwrap()));
    }

    #[test]
    fn tickets_and_nbt_round_trip() {
        let mut c = ChunkPois::default();
        let home = type_index("minecraft:home").unwrap();
        c.add([3, 70, -5], home);
        assert!(!c.is_village_center(4));
        assert!(c.acquire([3, 70, -5]));
        assert!(!c.acquire([3, 70, -5]));
        assert!(c.is_village_center(4));
        let tag = c.to_nbt(4000);
        let back = ChunkPois::from_nbt(&tag);
        assert_eq!(back.sections, c.sections);
        c.remove([3, 70, -5]);
        assert!(c.get([3, 70, -5]).is_none());
    }
}
