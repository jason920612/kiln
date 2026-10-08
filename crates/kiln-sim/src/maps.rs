//! Maps (`MapItem`, `MapItemSavedData`): a filled map is an item with a map id; the id names the
//! server-wide saved data (128 by 128 colours, the scale, the centre, markers) that holders of the map
//! keep up to date by looking at the world around them and that every holder is sent as
//! `map_item_data` packets.
//!
//! This module is the data and its rules; the places that use them are the player tick (a held
//! map redraws and sends what changed, [`crate::Sim::tick_maps`]), the empty map ([`use_empty_map`]),
//! banners ([`use_on_banner`]), item frames and the cartography table.

use bytes::{BufMut, Bytes, BytesMut};
use kiln_item::ItemStack;
use kiln_proto::codec::WriteExt;
use kiln_proto::nbt::Tag;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// `MapItemSavedData.MAP_SIZE`.
const SIZE: i32 = 128;
const PIXELS: usize = 128 * 128;
/// `MapItemSavedData.TRACKED_DECORATION_LIMIT`.
const TRACKED_LIMIT: i32 = 256;

// `MapDecorationTypes` by `minecraft:map_decoration_type` id.
pub(crate) const PLAYER: i32 = 0;
pub(crate) const FRAME: i32 = 1;
pub(crate) const PLAYER_OFF_MAP: i32 = 6;
pub(crate) const PLAYER_OFF_LIMITS: i32 = 7;
const BANNER_FIRST: i32 = 10;

/// `MapDecorationType.trackCount`: players, frames, markers and banners count toward the limit.
fn track_count(kind: i32) -> bool {
    matches!(kind, 0 | 1 | 2 | 3 | 6 | 7 | 10..=25)
}

/// `MapColor` ids the rendering uses.
const WATER: u8 = 12;
const DIRT: u8 = 10;
const STONE: u8 = 11;
const FIRE: u8 = 4;

/// `MapColor.Brightness` ids.
const LOW: u8 = 0;
const NORMAL: u8 = 1;
const HIGH: u8 = 2;
#[allow(dead_code)]
pub(crate) const LOWEST: u8 = 3;

/// `DyeColor` names by id (banners).
pub(crate) const DYES: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black",
];

/// One marker on a map (`MapDecoration`): the type, the position in half pixels from the centre (-128
/// to 127), the rotation (0 to 15) and a name.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Deco {
    pub kind: i32,
    pub x: i8,
    pub y: i8,
    pub rot: i8,
    pub name: Option<Tag>,
}

/// `MapBanner`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Banner {
    pub pos: [i32; 3],
    pub color: u8,
    pub name: Option<Tag>,
}

impl Banner {
    fn id(&self) -> String {
        format!("banner-{},{},{}", self.pos[0], self.pos[1], self.pos[2])
    }
}

/// `MapFrame`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Frame {
    pub pos: [i32; 3],
    pub rotation: i32,
    pub entity_id: i32,
}

fn frame_id(pos: [i32; 3]) -> String {
    format!("frame-{},{},{}", pos[0], pos[1], pos[2])
}

fn frame_key(entity_id: i32) -> String {
    format!("frame-{entity_id}")
}

/// What the map sends one of its holders (`MapItemSavedData.HoldingPlayer`).
#[derive(Debug, Clone)]
struct Holder {
    uuid: Uuid,
    name: String,
    dirty_data: bool,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
    dirty_decorations: bool,
    tick: i32,
    /// Counts the map's redraws for this player: a column is redrawn every 16th.
    step: i32,
}

impl Holder {
    fn new(uuid: Uuid, name: &str) -> Holder {
        Holder { uuid, name: name.to_owned(), dirty_data: true, min_x: 0, min_y: 0, max_x: 127, max_y: 127, dirty_decorations: true, tick: 0, step: 0 }
    }

    fn mark_colors_dirty(&mut self, x: i32, y: i32) {
        if self.dirty_data {
            self.min_x = self.min_x.min(x);
            self.min_y = self.min_y.min(y);
            self.max_x = self.max_x.max(x);
            self.max_y = self.max_y.max(y);
        } else {
            self.dirty_data = true;
            self.min_x = x;
            self.min_y = y;
            self.max_x = x;
            self.max_y = y;
        }
    }
}

/// A player as the maps see one.
#[derive(Debug, Clone)]
pub(crate) struct Viewer {
    pub uuid: Uuid,
    pub name: String,
    pub dim: usize,
    pub pos: [f64; 3],
    pub yaw: f32,
    /// The ids of the filled maps anywhere in the inventory.
    pub maps: Vec<i32>,
    /// A worn item of `#minecraft:map_invisibility_equipment`.
    pub hidden: bool,
}

/// An item frame that holds the map being ticked.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FrameInfo {
    pub pos: [i32; 3],
    /// `Direction.get2DDataValue()` of the frame's facing.
    pub direction: i32,
    pub entity_id: i32,
}

/// `MapItemSavedData.MapDecorationLocation`.
struct Location {
    kind: i32,
    x: i8,
    y: i8,
    rot: i8,
}

/// The world as `MapItem.update` reads it.
pub(crate) trait MapWorld {
    fn min_y(&self) -> i32;
    fn has_ceiling(&self) -> bool;
    /// Whether the chunk is there to read (vanilla would load it).
    fn loaded(&self, cx: i32, cz: i32) -> bool;
    /// `Heightmap.Types.WORLD_SURFACE`: the y above the column's highest block that is not air.
    fn surface(&self, x: i32, z: i32) -> i32;
    fn block(&self, x: i32, y: i32, z: i32) -> u16;
    /// `MapBanner.fromWorld`: the banner block entity at the position, as (dye id, custom name).
    fn banner(&self, x: i32, y: i32, z: i32) -> Option<(u8, Option<Tag>)>;
}

/// A map's saved data.
#[derive(Debug, Clone)]
pub(crate) struct MapData {
    pub dimension: String,
    pub center: [i32; 2],
    pub scale: i8,
    pub tracking_position: bool,
    pub unlimited_tracking: bool,
    pub locked: bool,
    pub colors: Vec<u8>,
    banners: BTreeMap<String, Banner>,
    /// Insertion order (a `LinkedHashMap`).
    decorations: Vec<(String, Deco)>,
    frames: BTreeMap<String, Frame>,
    tracked: i32,
    holders: Vec<Holder>,
    /// Changed since it was saved.
    pub dirty: bool,
}

impl MapData {
    fn new(center: [i32; 2], scale: i8, tracking: bool, unlimited: bool, locked: bool, dimension: &str) -> MapData {
        MapData {
            dimension: dimension.to_owned(),
            center,
            scale,
            tracking_position: tracking,
            unlimited_tracking: unlimited,
            locked,
            colors: vec![0; PIXELS],
            banners: BTreeMap::new(),
            decorations: Vec::new(),
            frames: BTreeMap::new(),
            tracked: 0,
            holders: Vec::new(),
            dirty: true,
        }
    }

    /// `MapItemSavedData.createFresh`: a map for a player at (`x`, `z`); maps tile the world, so the
    /// centre snaps to the grid of the scale.
    pub(crate) fn create_fresh(x: f64, z: f64, scale: i8, tracking: bool, unlimited: bool, dimension: &str) -> MapData {
        let size = SIZE * (1 << scale);
        let i = ((x + 64.0) / size as f64).floor() as i32;
        let j = ((z + 64.0) / size as f64).floor() as i32;
        let cx = i.wrapping_mul(size) + size / 2 - 64;
        let cz = j.wrapping_mul(size) + size / 2 - 64;
        MapData::new([cx, cz], scale, tracking, unlimited, false, dimension)
    }

    /// `MapItemSavedData.locked`: a copy that no longer changes.
    pub(crate) fn locked_copy(&self) -> MapData {
        let mut m = MapData::new(self.center, self.scale, self.tracking_position, self.unlimited_tracking, true, &self.dimension);
        m.banners = self.banners.clone();
        m.decorations = self.decorations.clone();
        m.tracked = self.tracked;
        m.colors = self.colors.clone();
        m
    }

    /// `MapItemSavedData.scaled`: the same place at the next scale, blank.
    pub(crate) fn scaled(&self) -> MapData {
        let scale = (self.scale as i32 + 1).clamp(0, 4) as i8;
        MapData::create_fresh(self.center[0] as f64, self.center[1] as f64, scale, self.tracking_position, self.unlimited_tracking, &self.dimension)
    }

    fn holder_index(&mut self, uuid: Uuid, name: &str) -> usize {
        match self.holders.iter().position(|h| h.uuid == uuid) {
            Some(i) => i,
            None => {
                self.holders.push(Holder::new(uuid, name));
                self.holders.len() - 1
            }
        }
    }

    fn set_colors_dirty(&mut self, x: i32, y: i32) {
        self.dirty = true;
        for h in &mut self.holders {
            h.mark_colors_dirty(x, y);
        }
    }

    fn set_decorations_dirty(&mut self) {
        for h in &mut self.holders {
            h.dirty_decorations = true;
        }
    }

    /// `setColor`.
    pub(crate) fn set_color(&mut self, x: i32, y: i32, color: u8) {
        self.colors[(x + y * SIZE) as usize] = color;
        self.set_colors_dirty(x, y);
    }

    /// `updateColor`: whether it changed.
    fn update_color(&mut self, x: i32, y: i32, color: u8) -> bool {
        if self.colors[(x + y * SIZE) as usize] != color {
            self.set_color(x, y, color);
            true
        } else {
            false
        }
    }

    pub(crate) fn is_tracked_count_over_limit(&self, limit: i32) -> bool {
        self.tracked > limit
    }

    pub(crate) fn decorations(&self) -> impl Iterator<Item = &Deco> {
        self.decorations.iter().map(|(_, d)| d)
    }

    pub(crate) fn has_decoration(&self, key: &str) -> bool {
        self.decorations.iter().any(|(k, _)| k == key)
    }

    fn remove_decoration(&mut self, key: &str) {
        if let Some(i) = self.decorations.iter().position(|(k, _)| k == key) {
            let (_, d) = self.decorations.remove(i);
            if track_count(d.kind) {
                self.tracked -= 1;
            }
        }
        self.set_decorations_dirty();
    }

    /// `addDecoration`: the marker of an object at (`x`, `z`) in blocks facing `rot` degrees, as this map shows it.
    #[allow(clippy::too_many_arguments)]
    fn add_decoration(&mut self, kind: i32, game_time: Option<i64>, key: &str, x: f64, z: f64, rot: f64, name: Option<Tag>) {
        let size = (1i32 << self.scale) as f32;
        let fx = (x - self.center[0] as f64) as f32 / size;
        let fz = (z - self.center[1] as f64) as f32 / size;
        let Some(loc) = self.decoration_location(kind, game_time, rot, fx, fz) else {
            self.remove_decoration(key);
            return;
        };
        let deco = Deco { kind: loc.kind, x: loc.x, y: loc.y, rot: loc.rot, name };
        let old = match self.decorations.iter().position(|(k, _)| k == key) {
            Some(i) => Some(std::mem::replace(&mut self.decorations[i].1, deco.clone())),
            None => {
                self.decorations.push((key.to_owned(), deco.clone()));
                None
            }
        };
        if old.as_ref() != Some(&deco) {
            if old.as_ref().is_some_and(|o| track_count(o.kind)) {
                self.tracked -= 1;
            }
            if track_count(loc.kind) {
                self.tracked += 1;
            }
            self.set_decorations_dirty();
        }
    }

    fn decoration_location(&self, kind: i32, game_time: Option<i64>, rot: f64, x: f32, z: f32) -> Option<Location> {
        let (b, c) = (clamp_coordinate(x), clamp_coordinate(z));
        if kind == PLAYER {
            let (kind, rot) = if inside(x, z) {
                (kind, self.rotation(game_time, rot))
            } else {
                // `decorationTypeForPlayerOutsideMap`
                let t = if x.abs() < 320.0 && z.abs() < 320.0 {
                    PLAYER_OFF_MAP
                } else if self.unlimited_tracking {
                    PLAYER_OFF_LIMITS
                } else {
                    return None;
                };
                (t, self.rotation(game_time, rot))
            };
            return Some(Location { kind, x: b, y: c, rot });
        }
        if inside(x, z) || self.unlimited_tracking {
            return Some(Location { kind, x: b, y: c, rot: self.rotation(game_time, rot) });
        }
        None
    }

    /// `calculateRotation`: the Nether's maps spin.
    fn rotation(&self, game_time: Option<i64>, rot: f64) -> i8 {
        if self.dimension == "minecraft:the_nether"
            && let Some(t) = game_time
        {
            let i = (t / 10) as i32;
            return ((i.wrapping_mul(i).wrapping_mul(34187121).wrapping_add(i.wrapping_mul(121)) >> 15) & 15) as i8;
        }
        let d = if rot < 0.0 { rot - 8.0 } else { rot + 8.0 };
        (d * 16.0 / 360.0) as i32 as i8
    }

    /// `MapItemSavedData.addTargetDecoration` happens on the stack, see [`add_target_decoration`].
    ///
    /// `tickCarriedBy`: `viewer` has the map `map_id` (in `stack`, or the item frame `frame`) at this tick.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tick_carried_by(
        &mut self,
        viewer: &Viewer,
        map_id: i32,
        stack: &ItemStack,
        frame: Option<FrameInfo>,
        game_time: i64,
        lookup: &dyn Fn(Uuid) -> Option<Viewer>,
    ) {
        self.holder_index(viewer.uuid, &viewer.name);
        if !viewer.maps.contains(&map_id) {
            self.remove_decoration(&viewer.name);
        }
        let mut i = 0;
        while i < self.holders.len() {
            let (uuid, name) = (self.holders[i].uuid, self.holders[i].name.clone());
            let other = lookup(uuid);
            let gone = match &other {
                None => true,
                Some(o) => frame.is_none() && !o.maps.contains(&map_id),
            };
            if gone {
                self.holders.remove(i);
                self.remove_decoration(&name);
            } else if let Some(o) = &other
                && frame.is_none()
                && DIM_NAMES.get(o.dim).copied() == Some(self.dimension.as_str())
                && self.tracking_position
            {
                self.add_decoration(PLAYER, Some(game_time), &name, o.pos[0], o.pos[2], o.yaw as f64, None);
            }
            if let Some(o) = &other
                && o.uuid != viewer.uuid
                && o.hidden
            {
                self.remove_decoration(&o.name);
            }
            i += 1;
        }
        if let Some(f) = frame
            && self.tracking_position
        {
            let key = frame_id(f.pos);
            if let Some(old) = self.frames.get(&key).cloned()
                && f.entity_id != old.entity_id
                && self.frames.contains_key(&frame_id(old.pos))
            {
                self.remove_decoration(&frame_key(old.entity_id));
            }
            let fresh = Frame { pos: f.pos, rotation: f.direction * 90, entity_id: f.entity_id };
            self.add_decoration(FRAME, Some(game_time), &frame_key(f.entity_id), f.pos[0] as f64, f.pos[2] as f64, (f.direction * 90) as f64, None);
            let old = self.frames.insert(fresh.id_string(), fresh.clone());
            if old.as_ref() != Some(&fresh) {
                self.dirty = true;
            }
        }
        // The decorations the item itself lists (exploration maps).
        if let Some(listed) = stack.get(kiln_item::keys::MAP_DECORATIONS) {
            if !listed.0.iter().all(|(k, _)| self.has_decoration(k)) {
                for (k, d) in &listed.0 {
                    if !self.has_decoration(k) {
                        self.add_decoration(d.kind, Some(game_time), k, d.x, d.z, d.rotation as f64, None);
                    }
                }
            }
        }
    }

    /// `getUpdatePacket`: what changed since the last one, for a holder (`None`: nothing, or not a holder).
    pub(crate) fn update_packet(&mut self, map_id: i32, uuid: Uuid) -> Option<Bytes> {
        let h = self.holders.iter().position(|h| h.uuid == uuid)?;
        let scale = self.scale;
        let locked = self.locked;
        let patch = if self.holders[h].dirty_data {
            self.holders[h].dirty_data = false;
            let hp = &self.holders[h];
            let (x0, y0) = (hp.min_x, hp.min_y);
            let (w, hgt) = (hp.max_x + 1 - hp.min_x, hp.max_y + 1 - hp.min_y);
            // (`createPatch`: index `i + j * w`.)
            let mut colors = vec![0u8; (w * hgt) as usize];
            for i in 0..w {
                for j in 0..hgt {
                    colors[(i + j * w) as usize] = self.colors[((x0 + i) + (y0 + j) * SIZE) as usize];
                }
            }
            Some((x0, y0, w, hgt, colors))
        } else {
            None
        };
        let decorations = {
            let hp = &mut self.holders[h];
            if hp.dirty_decorations && {
                let due = hp.tick % 5 == 0;
                hp.tick += 1;
                due
            } {
                hp.dirty_decorations = false;
                Some(self.decorations.iter().map(|(_, d)| d.clone()).collect::<Vec<_>>())
            } else {
                None
            }
        };
        if decorations.is_none() && patch.is_none() {
            return None;
        }
        Some(encode_packet(map_id, scale, locked, decorations.as_deref(), patch))
    }

    /// `toggleBanner`: a banner at `pos` is added to the map or, if the map has it, removed.
    pub(crate) fn toggle_banner(&mut self, world: &dyn MapWorld, pos: [i32; 3], game_time: i64) -> bool {
        let (x, z) = (pos[0] as f64 + 0.5, pos[2] as f64 + 0.5);
        let size = (1i32 << self.scale) as f64;
        let (d, e) = ((x - self.center[0] as f64) / size, (z - self.center[1] as f64) / size);
        if !(-63.0..=63.0).contains(&d) || !(-63.0..=63.0).contains(&e) {
            return false;
        }
        let Some(banner) = banner_at(world, pos) else { return false };
        let id = banner.id();
        if self.banners.get(&id) == Some(&banner) {
            self.banners.remove(&id);
            self.remove_decoration(&id);
            self.dirty = true;
            return true;
        }
        if !self.is_tracked_count_over_limit(TRACKED_LIMIT) {
            self.banners.insert(id.clone(), banner.clone());
            self.add_decoration(BANNER_FIRST + banner.color as i32, Some(game_time), &id, x, z, 180.0, banner.name.clone());
            self.dirty = true;
            return true;
        }
        false
    }

    /// `checkBanners`: markers of banners at the column that are gone or changed go.
    fn check_banners(&mut self, world: &dyn MapWorld, x: i32, z: i32) {
        let ids: Vec<String> = self.banners.iter().filter(|(_, b)| b.pos[0] == x && b.pos[2] == z).map(|(k, _)| k.clone()).collect();
        for id in ids {
            let b = self.banners[&id].clone();
            if banner_at(world, b.pos).as_ref() != Some(&b) {
                self.banners.remove(&id);
                self.remove_decoration(&id);
                self.dirty = true;
            }
        }
    }

    /// `removedFromFrame`.
    pub(crate) fn removed_from_frame(&mut self, pos: [i32; 3], entity_id: i32) {
        self.remove_decoration(&frame_key(entity_id));
        self.frames.remove(&frame_id(pos));
        self.dirty = true;
    }

    /// The saved form (`MapItemSavedData.CODEC`).
    pub(crate) fn to_nbt(&self) -> Tag {
        let banners = self
            .banners
            .values()
            .map(|b| {
                let mut f = vec![("pos".to_owned(), Tag::IntArray(b.pos.to_vec())), ("color".to_owned(), Tag::String(DYES[b.color as usize].to_owned()))];
                if let Some(n) = &b.name {
                    f.push(("name".to_owned(), n.clone()));
                }
                Tag::Compound(f)
            })
            .collect();
        let frames = self
            .frames
            .values()
            .map(|f| {
                Tag::Compound(vec![
                    ("pos".to_owned(), Tag::IntArray(f.pos.to_vec())),
                    ("rotation".to_owned(), Tag::Int(f.rotation)),
                    ("entity_id".to_owned(), Tag::Int(f.entity_id)),
                ])
            })
            .collect();
        Tag::Compound(vec![
            ("dimension".to_owned(), Tag::String(self.dimension.clone())),
            ("xCenter".to_owned(), Tag::Int(self.center[0])),
            ("zCenter".to_owned(), Tag::Int(self.center[1])),
            ("scale".to_owned(), Tag::Byte(self.scale)),
            ("colors".to_owned(), Tag::ByteArray(self.colors.iter().map(|&c| c as i8).collect())),
            ("trackingPosition".to_owned(), Tag::Byte(self.tracking_position as i8)),
            ("unlimitedTracking".to_owned(), Tag::Byte(self.unlimited_tracking as i8)),
            ("locked".to_owned(), Tag::Byte(self.locked as i8)),
            ("banners".to_owned(), Tag::List(banners)),
            ("frames".to_owned(), Tag::List(frames)),
        ])
    }

    pub(crate) fn from_nbt(t: &Tag) -> Option<MapData> {
        let int = |k: &str| t.get(k).and_then(Tag::as_i64);
        let flag = |k: &str, d: bool| t.get(k).and_then(Tag::as_i64).map_or(d, |v| v != 0);
        let dimension = t.get("dimension")?.as_str()?.to_owned();
        let scale = (int("scale").unwrap_or(0) as i8).clamp(0, 4);
        let mut m = MapData::new(
            [int("xCenter")? as i32, int("zCenter")? as i32],
            scale,
            flag("trackingPosition", true),
            flag("unlimitedTracking", false),
            flag("locked", false),
            &dimension,
        );
        if let Some(c) = t.get("colors").and_then(Tag::as_byte_array)
            && c.len() == PIXELS
        {
            m.colors = c.iter().map(|&b| b as u8).collect();
        }
        let pos_of = |e: &Tag| match e.get("pos") {
            Some(Tag::IntArray(p)) if p.len() == 3 => Some([p[0], p[1], p[2]]),
            _ => None,
        };
        for b in t.get("banners").and_then(Tag::as_list).unwrap_or(&[]) {
            let Some(pos) = pos_of(b) else { continue };
            let color = b.get("color").and_then(Tag::as_str).and_then(|c| DYES.iter().position(|d| *d == c)).unwrap_or(0) as u8;
            let banner = Banner { pos, color, name: b.get("name").cloned() };
            m.banners.insert(banner.id(), banner.clone());
            m.add_decoration(BANNER_FIRST + color as i32, None, &banner.id(), pos[0] as f64, pos[2] as f64, 180.0, banner.name);
        }
        for f in t.get("frames").and_then(Tag::as_list).unwrap_or(&[]) {
            let Some(pos) = pos_of(f) else { continue };
            let frame = Frame { pos, rotation: f.get("rotation").and_then(Tag::as_i64).unwrap_or(0) as i32, entity_id: f.get("entity_id").and_then(Tag::as_i64).unwrap_or(0) as i32 };
            m.frames.insert(frame.id_string(), frame.clone());
            m.add_decoration(FRAME, None, &frame_key(frame.entity_id), pos[0] as f64, pos[2] as f64, frame.rotation as f64, None);
        }
        m.dirty = false;
        Some(m)
    }

    /// `MapItem.update`: `viewer` holds the map in a hand; redraws the columns it is due.
    pub(crate) fn update(&mut self, world: &dyn MapWorld, viewer: &Viewer) {
        if DIM_NAMES.get(viewer.dim).copied() != Some(self.dimension.as_str()) {
            return;
        }
        let sb = 1i32 << self.scale;
        let [cx, cz] = self.center;
        let px = ((viewer.pos[0] - cx as f64).floor() as i32) / sb + 64;
        let pz = ((viewer.pos[2] - cz as f64).floor() as i32) / sb + 64;
        let mut r = SIZE / sb;
        let ceiling = world.has_ceiling();
        if ceiling {
            r /= 2;
        }
        let hi = self.holder_index(viewer.uuid, &viewer.name);
        self.holders[hi].step += 1;
        let step = self.holders[hi].step;
        let min_y = world.min_y();
        let mut forced = false;
        for x in (px - r + 1)..(px + r) {
            if (x & 15) != (step & 15) && !forced {
                continue;
            }
            forced = false;
            let mut prev_h = 0.0f64;
            for z in (pz - r - 1)..(pz + r) {
                if x < 0 || z < -1 || x >= SIZE || z >= SIZE {
                    continue;
                }
                let dist2 = (x - px) * (x - px) + (z - pz) * (z - pz);
                let ring = dist2 > (r - 2) * (r - 2);
                let block_x = (cx / sb + x - 64).wrapping_mul(sb);
                let block_z = (cz / sb + z - 64).wrapping_mul(sb);
                if !world.loaded(block_x >> 4, block_z >> 4) {
                    continue;
                }
                // The colours seen in the pixel, counted, first seen first.
                let mut seen: Vec<(u8, i32)> = Vec::new();
                let mut add = |c: u8, n: i32| match seen.iter_mut().find(|(k, _)| *k == c) {
                    Some(e) => e.1 += n,
                    None => seen.push((c, n)),
                };
                let mut liquid = 0i32;
                let mut height_sum = 0.0f64;
                if ceiling {
                    let mut h = block_x.wrapping_add(block_z.wrapping_mul(231871));
                    h = h.wrapping_mul(h).wrapping_mul(31287121).wrapping_add(h.wrapping_mul(11));
                    if (h >> 20) & 1 == 0 {
                        add(DIRT, 10);
                    } else {
                        add(STONE, 100);
                    }
                    height_sum = 100.0;
                } else {
                    for dx in 0..sb {
                        for dz in 0..sb {
                            let (bx, bz) = (block_x + dx, block_z + dz);
                            let mut y = world.surface(bx, bz) + 1;
                            let mut state;
                            if y > min_y {
                                loop {
                                    y -= 1;
                                    state = world.block(bx, y, bz);
                                    if kiln_data::map_color(state) != 0 || y <= min_y {
                                        break;
                                    }
                                }
                                if y > min_y && !kiln_data::block_logic::fluid(state).is_empty() {
                                    let mut k = y - 1;
                                    loop {
                                        let below = world.block(bx, k, bz);
                                        k -= 1;
                                        liquid += 1;
                                        if !(k > min_y && !kiln_data::block_logic::fluid(below).is_empty()) {
                                            break;
                                        }
                                    }
                                    state = correct_fluid_state(state);
                                }
                            } else {
                                state = kiln_data::blocks::default_state::BEDROCK;
                            }
                            self.check_banners(world, bx, bz);
                            height_sum += y as f64 / (sb * sb) as f64;
                            add(color_of(state), 1);
                        }
                    }
                }
                liquid /= sb * sb;
                // `Multisets.copyHighestCountFirst`: most seen first, a tie keeps the first seen first.
                let mut by_count = seen.clone();
                by_count.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
                let color = by_count.first().map_or(0, |&(c, _)| c);
                let parity = ((x + z) & 1) as f64;
                let brightness = if color == WATER {
                    let d = liquid as f64 * 0.1 + parity * 0.2;
                    if d < 0.5 {
                        HIGH
                    } else if d > 0.9 {
                        LOW
                    } else {
                        NORMAL
                    }
                } else {
                    let d = (height_sum - prev_h) * 4.0 / (sb + 4) as f64 + (parity - 0.5) * 0.4;
                    if d > 0.6 {
                        HIGH
                    } else if d < -0.6 {
                        LOW
                    } else {
                        NORMAL
                    }
                };
                prev_h = height_sum;
                if z >= 0 && dist2 < r * r && (!ring || ((x + z) & 1) != 0) {
                    forced |= self.update_color(x, z, color.wrapping_mul(4).wrapping_add(brightness));
                }
            }
        }
    }
}

impl Frame {
    fn id_string(&self) -> String {
        frame_id(self.pos)
    }
}

/// The names of the levels, by [`crate::DimId`].
const DIM_NAMES: [&str; 3] = ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"];

/// `isInsideMap`.
fn inside(x: f32, z: f32) -> bool {
    (-63.0..=63.0).contains(&x) && (-63.0..=63.0).contains(&z)
}

/// `clampMapCoordinate`.
fn clamp_coordinate(f: f32) -> i8 {
    if f <= -63.0 {
        -128
    } else if f >= 63.0 {
        127
    } else {
        (f as f64 * 2.0 + 0.5) as i32 as i8
    }
}

/// `BlockState.getMapColor`.
fn color_of(state: u16) -> u8 {
    kiln_data::map_color(state)
}

/// `getCorrectStateForFluidBlock`'s colour: a block under a fluid that cannot hold it up shows the fluid.
fn correct_fluid_state(state: u16) -> u16 {
    use kiln_data::block_logic::{FluidKind, Support, face_sturdy, fluid};
    let f = fluid(state);
    if !f.is_empty() && !face_sturdy(state, 1, Support::Full) {
        return match f.kind {
            FluidKind::Lava => kiln_data::blocks::default_state::LAVA,
            _ => kiln_data::blocks::default_state::WATER,
        };
    }
    state
}

/// `MapBanner.fromWorld`.
fn banner_at(world: &dyn MapWorld, pos: [i32; 3]) -> Option<Banner> {
    world.banner(pos[0], pos[1], pos[2]).map(|(color, name)| Banner { pos, color, name })
}

/// The packet (`ClientboundMapItemDataPacket`).
fn encode_packet(map_id: i32, scale: i8, locked: bool, decorations: Option<&[Deco]>, patch: Option<(i32, i32, i32, i32, Vec<u8>)>) -> Bytes {
    let mut b = BytesMut::new();
    b.put_varint(kiln_data::packets::play::clientbound::MAP_ITEM_DATA);
    b.put_varint(map_id);
    b.put_u8(scale as u8);
    b.put_bool(locked);
    match decorations {
        Some(list) => {
            b.put_bool(true);
            b.put_varint(list.len() as i32);
            for d in list {
                b.put_varint(d.kind);
                b.put_u8(d.x as u8);
                b.put_u8(d.y as u8);
                b.put_u8(d.rot as u8);
                match &d.name {
                    Some(n) => {
                        b.put_bool(true);
                        n.write_network(&mut b);
                    }
                    None => b.put_bool(false),
                }
            }
        }
        None => b.put_bool(false),
    }
    match patch {
        Some((x, y, w, h, colors)) => {
            b.put_u8(w as u8);
            b.put_u8(h as u8);
            b.put_u8(x as u8);
            b.put_u8(y as u8);
            b.put_varint(colors.len() as i32);
            b.put_slice(&colors);
        }
        None => b.put_u8(0),
    }
    b.freeze()
}

/// All maps of a server (`MapIndex` and the `maps/<id>` saved data).
#[derive(Default)]
pub(crate) struct MapStore {
    maps: BTreeMap<i32, MapData>,
    /// `MapIndex.lastMapId`, -1 before the first map.
    last_id: i32,
    index_dirty: bool,
    /// The world folder maps are saved under.
    dir: Option<std::path::PathBuf>,
}

pub(crate) type SharedMaps = Arc<Mutex<MapStore>>;

impl MapStore {
    pub(crate) fn new(dir: Option<std::path::PathBuf>) -> MapStore {
        let last_id = dir
            .as_deref()
            .and_then(|d| kiln_storage::saved_data::read(d, "maps/last_id"))
            .and_then(|t| t.get("map").and_then(Tag::as_i64))
            .map_or(-1, |v| v as i32);
        MapStore { maps: BTreeMap::new(), last_id, index_dirty: false, dir }
    }

    pub(crate) fn shared(dir: Option<std::path::PathBuf>) -> SharedMaps {
        Arc::new(Mutex::new(MapStore::new(dir)))
    }

    /// The id of the last map made (-1 before the first).
    pub(crate) fn last_id(&self) -> i32 {
        self.last_id
    }

    /// `ServerLevel.getFreeMapId`.
    pub(crate) fn free_id(&mut self) -> i32 {
        self.last_id += 1;
        self.index_dirty = true;
        self.last_id
    }

    /// `ServerLevel.setMapData`.
    pub(crate) fn set(&mut self, id: i32, data: MapData) {
        self.maps.insert(id, data);
    }

    /// `Level.getMapData`: loads the map from the world folder the first time.
    pub(crate) fn get(&mut self, id: i32) -> Option<&mut MapData> {
        if !self.maps.contains_key(&id) {
            let dir = self.dir.as_deref()?;
            let data = kiln_storage::saved_data::read(dir, &format!("maps/{id}"))?;
            self.maps.insert(id, MapData::from_nbt(&data)?);
        }
        self.maps.get_mut(&id)
    }

    /// The frames the maps in memory mark: (map id, level, block position, frame entity id).
    pub(crate) fn frame_markers(&self) -> Vec<(i32, String, [i32; 3], i32)> {
        self.maps.iter().flat_map(|(id, m)| m.frames.values().map(move |f| (*id, m.dimension.clone(), f.pos, f.entity_id))).collect()
    }

    /// Whether the map exists (in memory or saved).
    pub(crate) fn has(&mut self, id: i32) -> bool {
        self.get(id).is_some()
    }

    /// Writes the maps that changed.
    pub(crate) fn save(&mut self) {
        let Some(dir) = self.dir.clone() else { return };
        for (id, m) in self.maps.iter_mut().filter(|(_, m)| m.dirty) {
            if let Err(e) = kiln_storage::saved_data::write(&dir, &format!("maps/{id}"), m.to_nbt()) {
                tracing::warn!("failed to save map {id}: {e}");
            }
            m.dirty = false;
        }
        if self.index_dirty {
            let data = Tag::Compound(vec![("map".to_owned(), Tag::Int(self.last_id))]);
            if let Err(e) = kiln_storage::saved_data::write(&dir, "maps/last_id", data) {
                tracing::warn!("failed to save the map index: {e}");
            }
            self.index_dirty = false;
        }
    }
}

/// `MapItem.renderBiomePreviewMap`: the pixels of a map that has not been explored, drawn from the
/// biomes (`watery`: whether the biome of the block at (x, z) draws water on maps).
fn render_biome_preview(data: &mut MapData, watery: &mut dyn FnMut(i32, i32) -> bool) {
    let scale = 1i32 << data.scale;
    let (cx, cz) = (data.center[0], data.center[1]);
    let mut water = vec![false; PIXELS];
    let (x0, z0) = (cx / scale - 64, cz / scale - 64);
    for z in 0..128 {
        for x in 0..128 {
            water[(z * 128 + x) as usize] = watery((x0 + x) * scale, (z0 + z) * scale);
        }
    }
    let at = |x: i32, z: i32| water[(z * 128 + x) as usize];
    for i in 1..127 {
        for j in 1..127 {
            let mut count = 0;
            for k in -1..=1 {
                for l in -1..=1 {
                    if (k != 0 || l != 0) && at(i + k, j + l) {
                        count += 1;
                    }
                }
            }
            let mut brightness = LOWEST;
            let mut color = 0u8;
            if at(i, j) {
                color = 15; // COLOR_ORANGE
                if count > 7 && j % 2 == 0 {
                    // (`Mth.sin` is the table's.)
                    let phase = i + (kiln_javamath::mth::sin(j as f32 as f64) * 7.0f32) as i32;
                    brightness = match (phase / 8) % 5 {
                        0 | 4 => LOW,
                        1 | 3 => NORMAL,
                        2 => HIGH,
                        _ => brightness,
                    };
                } else if count > 7 {
                    color = 0;
                } else if count > 5 {
                    brightness = NORMAL;
                } else if count > 3 || count > 1 {
                    brightness = LOW;
                }
            } else if count > 0 {
                color = 26; // COLOR_BROWN
                brightness = if count > 3 { NORMAL } else { LOWEST };
            }
            if color != 0 {
                data.set_color(i, j, color.wrapping_mul(4).wrapping_add(brightness));
            }
        }
    }
}

/// The world part of `exploration_map` loot functions (`MapExplorer`): finds the nearest structure in the
/// overworld and makes the map for it.
pub(crate) struct Explorer {
    pub maps: SharedMaps,
    pub pipeline: Arc<kiln_worldgen::pipeline::Pipeline>,
    /// The biomes that draw water on maps (`#minecraft:water_on_map_outlines`), by `minecraft:worldgen/biome` id.
    pub watery: Vec<u16>,
}

impl Explorer {
    pub(crate) fn new(maps: SharedMaps, pipeline: Arc<kiln_worldgen::pipeline::Pipeline>) -> Explorer {
        let names = crate::world_state::worldgen_tag("worldgen/biome", "minecraft:water_on_map_outlines").unwrap_or_default();
        let watery = names.iter().filter_map(|n| kiln_data::synced_id("minecraft:worldgen/biome", n)).map(|i| i as u16).collect();
        Explorer { maps, pipeline, watery }
    }
}

impl kiln_loot::MapExplorer for Explorer {
    fn explore(&self, stack: &ItemStack, origin: [f64; 3], request: &kiln_loot::ExplorationMap<'_>) -> Option<ItemStack> {
        if stack.is_empty() {
            return None;
        }
        let at = [origin[0].floor() as i32, origin[1].floor() as i32, origin[2].floor() as i32];
        let names: Vec<String> = request.destination.names.iter().map(|n| n.to_string()).collect();
        let mut gs = kiln_worldgen::generator::GenScratch::default();
        let (target, _) = self.pipeline.find_nearest_structure(&mut gs, &names, at, request.search_radius)?;
        // `MapItem.applyNewSavedData(level, stack, x, z, zoom, true, true)`.
        let mut data = MapData::create_fresh(target[0] as f64, target[2] as f64, request.zoom, true, true, "minecraft:overworld");
        // `renderBiomePreviewMap` at the sea level.
        let world = self.pipeline.world().clone();
        let sea = world.generator.sea_level;
        let mut gs2 = kiln_worldgen::generator::GenScratch::default();
        let mut watery = |x: i32, z: i32| {
            let b = gs2.noise_biome(&world.generator, x >> 2, sea >> 2, z >> 2);
            self.watery.contains(&b)
        };
        render_biome_preview(&mut data, &mut watery);
        let mut store = self.maps.lock().unwrap_or_else(|e| e.into_inner());
        let id = store.free_id();
        store.set(id, data);
        let mut out = stack.clone();
        out.insert(kiln_item::keys::MAP_ID, kiln_item::component::MapId(id));
        add_target_decoration(&mut out, target, "+", request.decoration);
        Some(out)
    }
}

/// The map id of an item, if it is a filled map.
pub(crate) fn map_id_of(stack: &ItemStack) -> Option<i32> {
    stack.get(kiln_item::keys::MAP_ID).map(|m| m.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_loot::MapExplorer;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// `ExplorationMapFunction` against the vectors of `tools/ExploreMapVectors.java` (`KILN_EXPLORE_VECTORS`): the
    /// structure found, the map's centre, its biome preview and the marker on the item.
    #[test]
    fn exploration_map_parity() {
        let Some(path) = std::env::var_os("KILN_EXPLORE_VECTORS") else {
            eprintln!("skipped: set KILN_EXPLORE_VECTORS (tools/ExploreMapVectors.java)");
            return;
        };
        let seed: i64 = std::env::var("KILN_EXPLORE_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(12345);
        let pack = kiln_worldgen::Datapack::load(&crate::datapack_dir(None)).expect("datapack");
        let world = kiln_worldgen::Worldgen::overworld(&pack, seed, true).expect("overworld");
        let pipeline = Arc::new(kiln_worldgen::Pipeline::new(Arc::new(world)));
        let store = MapStore::shared(None);
        let explorer = Explorer::new(store.clone(), pipeline);
        let (mut checked, mut failed) = (0, Vec::new());
        for line in std::fs::read_to_string(path).unwrap().lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            if v.get("error").is_some() || v["found"].is_null() {
                continue;
            }
            let tag = v["tag"].as_str().unwrap();
            let names: Vec<kiln_item::Identifier> = crate::world_state::worldgen_tag("worldgen/structure", tag)
                .unwrap()
                .iter()
                .filter_map(|n| kiln_item::Identifier::parse(n))
                .collect();
            let destination = kiln_loot::parse::NameSet { tag: None, names };
            let kind = kiln_data::builtin_id("minecraft:map_decoration_type", &format!("minecraft:{}", v["decoration"].as_str().unwrap())).unwrap();
            let request = kiln_loot::ExplorationMap {
                destination: &destination,
                decoration: kind,
                zoom: v["zoom"].as_i64().unwrap() as i8,
                search_radius: v["radius"].as_i64().unwrap() as i32,
                skip_existing_chunks: v["skip"].as_bool().unwrap(),
            };
            let o = v["origin"].as_array().unwrap();
            let origin = [o[0].as_f64().unwrap(), 64.0, o[1].as_f64().unwrap()];
            let stack = ItemStack::of("minecraft:filled_map", 1).unwrap();
            let made = explorer.explore(&stack, origin, &request);
            checked += 1;
            let Some(made) = made else {
                failed.push(format!("{tag} at {origin:?}: nothing found, vanilla {}", v["found"]));
                continue;
            };
            let id = map_id_of(&made).unwrap();
            let mut s = store.lock().unwrap();
            let data = s.get(id).unwrap();
            let want_center = v["center"].as_array().unwrap();
            let comp = made.get(kiln_item::keys::MAP_DECORATIONS).unwrap().0.iter().find(|(k, _)| k == "+").unwrap().1.clone();
            let want_comp = v["component"].as_array().unwrap();
            let want_colors = unhex(v["colors"].as_str().unwrap());
            let diff = data.colors.iter().zip(&want_colors).filter(|(a, b)| a != b).count();
            if diff > 0 && std::env::var_os("KILN_EXPLORE_DEBUG").is_some() {
                let first: Vec<String> = data
                    .colors
                    .iter()
                    .zip(&want_colors)
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .take(12)
                    .map(|(i, (a, b))| format!("({},{}) kiln {a} vanilla {b}", i % 128, i / 128))
                    .collect();
                println!("{tag}: {first:?}");
            }
            if data.center != [want_center[0].as_i64().unwrap() as i32, want_center[1].as_i64().unwrap() as i32]
                || comp.kind as i64 != want_comp[0].as_i64().unwrap()
                || comp.x != want_comp[1].as_f64().unwrap()
                || comp.z != want_comp[2].as_f64().unwrap()
                || diff != 0
            {
                failed.push(format!("{tag} at {origin:?}: centre {:?} (vanilla {want_center:?}), marker {} {} (vanilla {want_comp:?}), {diff} colours differ", data.center, comp.x, comp.z));
            }
        }
        println!("exploration maps: {checked} checked, {} differ", failed.len());
        assert!(failed.is_empty(), "{failed:#?}");
    }
}

/// `MapItemSavedData.addTargetDecoration`: the map item shows a marker at `pos` named `key`.
pub(crate) fn add_target_decoration(stack: &mut ItemStack, pos: [i32; 3], key: &str, kind: i32) {
    let mut list = stack.get(kiln_item::keys::MAP_DECORATIONS).cloned().unwrap_or_default();
    let entry = kiln_item::component::MapDecoration { kind, x: pos[0] as f64, z: pos[2] as f64, rotation: 180.0 };
    match list.0.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = entry,
        None => list.0.push((key.to_owned(), entry)),
    }
    stack.insert(kiln_item::keys::MAP_DECORATIONS, list);
}
