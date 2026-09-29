//! Player data: `players/data/<uuid>.dat` (26.1+ layout), as vanilla `PlayerDataStorage`.
//!
//! The loaded compound is kept whole and only the fields Kiln models are overwritten on save,
//! so health, food, XP, effects, the ender chest, advancements of other mods and so on survive
//! a stay on Kiln. Items keep their saved NBT (components) while Kiln's view of the slot
//! (item id and count) is unchanged or only the count changed.

use crate::anvil::DATA_VERSION;
use crate::{child, put, read_nbt_file, remove, write_nbt_file};
use kiln_proto::nbt::Tag;
use std::path::{Path, PathBuf};
use tracing::warn;
use uuid::Uuid;

/// Player inventory menu slots Kiln tracks: 5-8 armor, 9-35 main, 36-44 hotbar, 45 offhand.
pub const INVENTORY_SLOTS: usize = 46;
const HOTBAR_START: usize = 36;
/// `equipment` keys stored in inventory menu slots.
const EQUIPMENT: [(&str, usize); 5] = [("head", 5), ("chest", 6), ("legs", 7), ("feet", 8), ("offhand", 45)];

/// An item as saved: Kiln's view (`id`, `count`) plus the saved compound (`id`, `count`,
/// `components`, ...). This is where a full item model (`kiln_item::ItemStack`) plugs in.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredItem {
    /// Protocol id in `minecraft:item`; `None` for an id this build does not know.
    pub id: Option<i32>,
    pub count: i32,
    pub nbt: Tag,
}

impl StoredItem {
    fn from_nbt(mut nbt: Tag) -> Option<Self> {
        remove(&mut nbt, "Slot");
        let name = nbt.get("id")?.as_str()?;
        let id = kiln_data::builtin_id("minecraft:item", name);
        if id.is_none() {
            warn!("unknown item {name} kept as saved");
        }
        let count = nbt.get("count").and_then(Tag::as_i64).unwrap_or(1) as i32;
        Some(Self { id, count, nbt })
    }

    fn new(id: i32, count: i32) -> Self {
        let name = kiln_data::builtin_entries("minecraft:item").and_then(|e| e.get(id as usize)).copied().unwrap_or("minecraft:air");
        let nbt = Tag::Compound(vec![("id".into(), Tag::String(name.to_owned())), ("count".into(), Tag::Int(count))]);
        Self { id: Some(id), count, nbt }
    }

    fn with_slot(&self, slot: i8) -> Tag {
        let mut t = self.nbt.clone();
        if let Tag::Compound(f) = &mut t {
            f.insert(0, ("Slot".into(), Tag::Byte(slot)));
        }
        t
    }
}

/// A player's saved state. Fields are what Kiln models; everything else stays in the loaded
/// compound.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerData {
    /// `Pos`; `None` for a file without one (vanilla then places the player at the spawn).
    pub pos: Option<[f64; 3]>,
    /// `Rotation` (yaw, pitch); `None` when absent (vanilla uses the world spawn's angles).
    pub rot: Option<[f32; 2]>,
    pub on_ground: bool,
    /// `playerGameType`; `None` when absent (vanilla uses the server's default game mode).
    pub game_mode: Option<u8>,
    /// `Dimension`.
    pub dimension: Option<String>,
    /// `SelectedItemSlot` (hotbar index).
    pub selected_slot: u8,
    /// Items by inventory menu slot (`INVENTORY_SLOTS` entries).
    pub inventory: Vec<Option<StoredItem>>,
    /// `respawn.pos` (the spawn point set by a bed or `/spawnpoint`).
    pub respawn: Option<[i32; 3]>,
    /// `respawn.dimension`; `None` is the overworld.
    pub respawn_dimension: Option<String>,
    raw: Tag,
}

impl Default for PlayerData {
    fn default() -> Self {
        Self {
            pos: None,
            rot: None,
            on_ground: false,
            game_mode: None,
            dimension: None,
            selected_slot: 0,
            inventory: vec![None; INVENTORY_SLOTS],
            respawn: None,
            respawn_dimension: None,
            raw: Tag::Compound(Vec::new()),
        }
    }
}

fn doubles<const N: usize>(tag: Option<&Tag>) -> Option<[f64; N]> {
    match tag?.as_list()? {
        l if l.len() == N => {
            let mut out = [0.0; N];
            for (o, t) in out.iter_mut().zip(l) {
                *o = match t {
                    Tag::Double(v) => *v,
                    Tag::Float(v) => *v as f64,
                    _ => return None,
                };
            }
            Some(out)
        }
        _ => None,
    }
}

impl PlayerData {
    /// Reads Kiln's fields from a vanilla player compound.
    pub fn from_nbt(raw: Tag) -> Self {
        let version = raw.get("DataVersion").and_then(Tag::as_i64).unwrap_or(0);
        if version != DATA_VERSION {
            warn!("player data version {version} differs from {DATA_VERSION}; fields may be read wrongly");
        }
        let mut inventory: Vec<Option<StoredItem>> = vec![None; INVENTORY_SLOTS];
        for entry in raw.get("Inventory").and_then(Tag::as_list).unwrap_or(&[]) {
            let Some(slot) = entry.get("Slot").and_then(Tag::as_i64) else { continue };
            let menu = match slot {
                0..=8 => HOTBAR_START + slot as usize,
                9..=35 => slot as usize,
                _ => continue, // vanilla ignores slots outside the 36 main slots
            };
            inventory[menu] = StoredItem::from_nbt(entry.clone());
        }
        if let Some(Tag::Compound(eq)) = raw.get("equipment") {
            for (key, menu) in EQUIPMENT {
                if let Some(item) = eq.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()) {
                    inventory[menu] = StoredItem::from_nbt(item);
                }
            }
        }
        let respawn = match raw.get("respawn").and_then(|r| r.get("pos")) {
            Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
            _ => None,
        };
        Self {
            pos: doubles::<3>(raw.get("Pos")),
            rot: doubles::<2>(raw.get("Rotation")).map(|[y, p]| [y as f32, p as f32]),
            on_ground: raw.get("OnGround").and_then(Tag::as_i64) == Some(1),
            game_mode: raw.get("playerGameType").and_then(Tag::as_i64).filter(|g| (0..=3).contains(g)).map(|g| g as u8),
            dimension: raw.get("Dimension").and_then(Tag::as_str).map(str::to_owned),
            selected_slot: raw.get("SelectedItemSlot").and_then(Tag::as_i64).filter(|s| (0..9).contains(s)).unwrap_or(0) as u8,
            inventory,
            respawn,
            respawn_dimension: raw.get("respawn").and_then(|r| r.get("dimension")).and_then(Tag::as_str).map(str::to_owned),
            raw,
        }
    }

    /// The loaded compound, for code that reads fields Kiln's model does not have.
    pub fn raw(&self) -> &Tag {
        &self.raw
    }

    /// The loaded compound, for fields Kiln keeps only in the saved form (`Tags`).
    pub fn raw_mut(&mut self) -> &mut Tag {
        if !matches!(self.raw, Tag::Compound(_)) {
            self.raw = Tag::Compound(Vec::new());
        }
        &mut self.raw
    }

    /// Kiln's view of the inventory: item id and count per menu slot (unknown items read as
    /// empty; they are kept on save while the slot stays empty).
    pub fn slots(&self) -> Vec<Option<(i32, i32)>> {
        self.inventory.iter().map(|s| s.as_ref().and_then(|i| Some((i.id?, i.count)))).collect()
    }

    /// Takes Kiln's inventory. A slot that still holds the same item keeps its saved NBT with
    /// the new count; a different item starts from a plain stack.
    pub fn set_slots(&mut self, slots: &[Option<(i32, i32)>]) {
        for (stored, &now) in self.inventory.iter_mut().zip(slots) {
            *stored = match (stored.take(), now) {
                (Some(s), None) if s.id.is_none() => Some(s),
                (_, None) => None,
                (Some(mut s), Some((id, count))) if s.id == Some(id) => {
                    if s.count != count {
                        s.count = count;
                        put(&mut s.nbt, "count", Tag::Int(count));
                    }
                    Some(s)
                }
                (_, Some((id, count))) => Some(StoredItem::new(id, count)),
            };
        }
    }

    /// The compound to save for the player `uuid`: the loaded one with Kiln's fields replaced.
    pub fn to_nbt(&self, uuid: Uuid) -> Tag {
        let mut t = self.raw.clone();
        put(&mut t, "DataVersion", Tag::Int(DATA_VERSION as i32));
        if let Some(p) = self.pos {
            put(&mut t, "Pos", Tag::List(p.iter().map(|&v| Tag::Double(v)).collect()));
            if t.get("Motion").is_none() {
                put(&mut t, "Motion", Tag::List(vec![Tag::Double(0.0); 3]));
            }
        }
        if let Some(r) = self.rot {
            put(&mut t, "Rotation", Tag::List(r.iter().map(|&v| Tag::Float(v)).collect()));
        }
        put(&mut t, "OnGround", Tag::Byte(self.on_ground as i8));
        let (hi, lo) = uuid.as_u64_pair();
        put(&mut t, "UUID", Tag::IntArray(vec![(hi >> 32) as i32, hi as i32, (lo >> 32) as i32, lo as i32]));
        if let Some(g) = self.game_mode {
            put(&mut t, "playerGameType", Tag::Int(g as i32));
        }
        if let Some(d) = &self.dimension {
            put(&mut t, "Dimension", Tag::String(d.clone()));
        }
        put(&mut t, "SelectedItemSlot", Tag::Int(self.selected_slot as i32));

        let main = (HOTBAR_START..HOTBAR_START + 9).map(|m| (m, (m - HOTBAR_START) as i8)).chain((9..36).map(|m| (m, m as i8)));
        let mut items: Vec<(i8, Tag)> =
            main.filter_map(|(menu, slot)| self.inventory[menu].as_ref().map(|i| (slot, i.with_slot(slot)))).collect();
        items.sort_by_key(|(slot, _)| *slot);
        put(&mut t, "Inventory", Tag::List(items.into_iter().map(|(_, i)| i).collect()));

        let equipment = child(&mut t, "equipment");
        for (key, menu) in EQUIPMENT {
            match &self.inventory[menu] {
                Some(item) => put(equipment, key, item.nbt.clone()),
                None => remove(equipment, key),
            }
        }
        if matches!(t.get("equipment"), Some(Tag::Compound(f)) if f.is_empty()) {
            remove(&mut t, "equipment");
        }

        let saved_respawn = match self.raw.get("respawn").and_then(|r| r.get("pos")) {
            Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
            _ => None,
        };
        let saved_dimension = self.raw.get("respawn").and_then(|r| r.get("dimension")).and_then(Tag::as_str);
        let dimension = self.respawn_dimension.as_deref().unwrap_or("minecraft:overworld");
        if self.respawn != saved_respawn || (self.respawn.is_some() && saved_dimension != Some(dimension)) {
            match self.respawn {
                Some(pos) => {
                    let r = Tag::Compound(vec![
                        ("dimension".into(), Tag::String(dimension.into())),
                        ("pos".into(), Tag::IntArray(pos.to_vec())),
                        ("yaw".into(), Tag::Float(0.0)),
                        ("pitch".into(), Tag::Float(0.0)),
                    ]);
                    put(&mut t, "respawn", r);
                }
                None => remove(&mut t, "respawn"),
            }
        }
        t
    }
}

/// `players/data` of a world.
pub struct PlayerStore {
    dir: PathBuf,
}

impl PlayerStore {
    pub fn new(world_dir: &Path) -> Self {
        Self { dir: world_dir.join("players/data") }
    }

    fn file(&self, uuid: Uuid, suffix: &str) -> PathBuf {
        self.dir.join(format!("{uuid}{suffix}"))
    }

    /// The player's saved data; like vanilla, an unreadable `<uuid>.dat` is copied aside as
    /// `<uuid>_corrupted_<date>.dat` and `<uuid>.dat_old` is tried instead.
    pub fn load(&self, uuid: Uuid) -> Option<PlayerData> {
        let path = self.file(uuid, ".dat");
        let raw = if path.is_file() {
            match read_nbt_file(&path) {
                Ok(t) => Some(t),
                Err(e) => {
                    warn!("failed to load player data {}: {e}", path.display());
                    let aside = self.file(uuid, &format!("_corrupted_{}.dat", file_date()));
                    if let Err(e) = std::fs::copy(&path, &aside) {
                        warn!("failed to copy {} aside: {e}", path.display());
                    }
                    None
                }
            }
        } else {
            None
        };
        let raw = raw.or_else(|| {
            let old = self.file(uuid, ".dat_old");
            old.is_file().then(|| read_nbt_file(&old).map_err(|e| warn!("failed to load {}: {e}", old.display())).ok())?
        })?;
        Some(PlayerData::from_nbt(raw))
    }

    /// Saves atomically, keeping the previous file as `<uuid>.dat_old`.
    pub fn save(&self, uuid: Uuid, data: &PlayerData) -> std::io::Result<()> {
        self.save_nbt(uuid, &data.to_nbt(uuid))
    }

    /// Writes a complete player compound (e.g. [`PlayerData::to_nbt`] with items written by a
    /// full item model).
    pub fn save_nbt(&self, uuid: Uuid, nbt: &Tag) -> std::io::Result<()> {
        write_nbt_file(&self.file(uuid, ".dat"), nbt, Some(&self.file(uuid, ".dat_old")))
    }
}

/// UTC time as `yyyy-MM-dd_HH.mm.ss` (vanilla names backups with local time in this form).
fn file_date() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    format!("{year:04}-{month:02}-{day:02}_{:02}.{:02}.{:02}", rem / 3600, rem / 60 % 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, count: i32, extra: Vec<(String, Tag)>) -> Tag {
        let mut f = vec![("id".to_owned(), Tag::String(name.into())), ("count".to_owned(), Tag::Int(count))];
        f.extend(extra);
        Tag::Compound(f)
    }

    fn slot(slot: i8, t: Tag) -> Tag {
        let Tag::Compound(mut f) = t else { unreachable!() };
        f.insert(0, ("Slot".into(), Tag::Byte(slot)));
        Tag::Compound(f)
    }

    fn sample() -> Tag {
        let enchanted = vec![(
            "components".to_owned(),
            Tag::Compound(vec![("minecraft:enchantments".into(), Tag::Compound(vec![("minecraft:sharpness".into(), Tag::Int(3))]))]),
        )];
        Tag::Compound(vec![
            ("DataVersion".into(), Tag::Int(5023)),
            ("Pos".into(), Tag::List(vec![Tag::Double(1.5), Tag::Double(70.0), Tag::Double(-2.25)])),
            ("Rotation".into(), Tag::List(vec![Tag::Float(45.0), Tag::Float(10.0)])),
            ("Health".into(), Tag::Float(7.5)),
            ("playerGameType".into(), Tag::Int(1)),
            ("SelectedItemSlot".into(), Tag::Int(2)),
            ("Dimension".into(), Tag::String("minecraft:overworld".into())),
            (
                "Inventory".into(),
                Tag::List(vec![
                    slot(0, item("minecraft:diamond_sword", 1, enchanted)),
                    slot(9, item("minecraft:stone", 32, vec![])),
                    slot(10, item("minecraft:not_an_item", 1, vec![])),
                ]),
            ),
            (
                "equipment".into(),
                Tag::Compound(vec![
                    ("head".into(), item("minecraft:diamond_helmet", 1, vec![])),
                    ("offhand".into(), item("minecraft:shield", 1, vec![])),
                    ("body".into(), item("minecraft:stone", 1, vec![])),
                ]),
            ),
        ])
    }

    #[test]
    fn reads_vanilla_fields() {
        let d = PlayerData::from_nbt(sample());
        assert_eq!(d.pos, Some([1.5, 70.0, -2.25]));
        assert_eq!(d.rot, Some([45.0, 10.0]));
        assert_eq!((d.game_mode, d.selected_slot), (Some(1), 2));
        let id = |n| kiln_data::builtin_id("minecraft:item", n).unwrap();
        let slots = d.slots();
        assert_eq!(slots[36], Some((id("minecraft:diamond_sword"), 1)));
        assert_eq!(slots[9], Some((id("minecraft:stone"), 32)));
        assert_eq!(slots[10], None, "unknown items read as empty");
        assert_eq!(slots[5], Some((id("minecraft:diamond_helmet"), 1)));
        assert_eq!(slots[45], Some((id("minecraft:shield"), 1)));
    }

    #[test]
    fn save_keeps_what_kiln_does_not_model() {
        let uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        let mut d = PlayerData::from_nbt(sample());
        assert_eq!(d.to_nbt(uuid).get("Inventory"), sample().get("Inventory"), "unchanged inventory saves as loaded");

        let id = |n| kiln_data::builtin_id("minecraft:item", n).unwrap();
        let mut slots = d.slots();
        slots[36] = Some((id("minecraft:diamond_sword"), 2)); // same item, new count: components stay
        slots[9] = Some((id("minecraft:dirt"), 5)); // a different item
        slots[45] = None;
        d.set_slots(&slots);
        d.pos = Some([10.0, 64.0, 10.0]);
        d.game_mode = Some(0);
        d.respawn = Some([1, 2, 3]);
        let t = d.to_nbt(uuid);

        assert_eq!(t.get("Health"), Some(&Tag::Float(7.5)));
        assert_eq!(t.get("playerGameType"), Some(&Tag::Int(0)));
        let inv = t.get("Inventory").and_then(Tag::as_list).unwrap();
        assert_eq!(inv.len(), 3);
        assert_eq!(inv[0].get("count"), Some(&Tag::Int(2)));
        assert!(inv[0].get("components").is_some());
        assert_eq!(inv[1], slot(9, item("minecraft:dirt", 5, vec![])));
        assert_eq!(inv[2].get("id").and_then(Tag::as_str), Some("minecraft:not_an_item"));
        let eq = t.get("equipment").unwrap();
        assert!(eq.get("offhand").is_none() && eq.get("head").is_some() && eq.get("body").is_some());
        assert_eq!(t.get("UUID"), Some(&Tag::IntArray(vec![0x0123_4567, 0x89ab_cdef_u32 as i32, 0x0123_4567, 0x89ab_cdef_u32 as i32])));
        assert_eq!(t.get("respawn").and_then(|r| r.get("pos")), Some(&Tag::IntArray(vec![1, 2, 3])));
        assert_eq!(PlayerData::from_nbt(t.clone()).to_nbt(uuid), t, "a second round trip changes nothing");
    }

    #[test]
    fn store_saves_atomically_and_falls_back_to_old() {
        let dir = std::env::temp_dir().join(format!("kiln-players-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = PlayerStore::new(&dir);
        let uuid = Uuid::from_u128(7);
        assert!(store.load(uuid).is_none());
        let mut d = PlayerData { pos: Some([0.5, 1.0, 0.5]), game_mode: Some(1), ..PlayerData::default() };
        store.save(uuid, &d).unwrap();
        d.pos = Some([2.5, 1.0, 0.5]);
        store.save(uuid, &d).unwrap();
        assert_eq!(store.load(uuid).unwrap().pos, Some([2.5, 1.0, 0.5]));
        // A corrupt file is set aside and the previous save used.
        std::fs::write(store.file(uuid, ".dat"), b"garbage").unwrap();
        assert_eq!(store.load(uuid).unwrap().pos, Some([0.5, 1.0, 0.5]));
        let aside = std::fs::read_dir(dir.join("players/data")).unwrap().filter(|e| {
            e.as_ref().unwrap().file_name().to_string_lossy().contains("_corrupted_")
        });
        assert_eq!(aside.count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn backup_dates() {
        let d = file_date();
        assert_eq!(d.len(), 19);
        assert_eq!(&d[4..5], "-");
        assert_eq!(&d[10..11], "_");
    }
}
