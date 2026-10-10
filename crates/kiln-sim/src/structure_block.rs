//! Structure blocks and jigsaw blocks (`StructureBlock`, `StructureBlockEntity`, `JigsawBlock`, `JigsawBlockEntity`).
//!
//! A structure block remembers a name, an offset and a size and does one of four things with them: in save mode it
//! copies the blocks of its area into a template (`StructureTemplate.fillFromWorld`) kept by the level's template
//! manager and written to `generated/<namespace>/structures/<path>.nbt`; in load mode it places a template back
//! (`StructureTemplate.placeInWorld`); corner blocks mark an area for a save block to measure; data blocks only
//! carry a string. The settings come from the screen (`ServerboundSetStructureBlockPacket`), redstone power saves or
//! loads without the screen. A jigsaw block's screen sets its fields and asks for a piece of the pool it names to be
//! put next to it (`JigsawPlacement.generateJigsaw`, the `/place jigsaw` machinery).
//!
//! The block entities live in the regions (`container`); what touches several chunks (the templates, the placing,
//! the scanning) runs in the serial phase, like the command blocks.

use kiln_proto::nbt::Tag;

/// `StructureMode`, in vanilla's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Save,
    Load,
    Corner,
    Data,
}

impl Mode {
    /// The `mode` block state value.
    pub fn state_name(self) -> &'static str {
        match self {
            Mode::Save => "save",
            Mode::Load => "load",
            Mode::Corner => "corner",
            Mode::Data => "data",
        }
    }

    pub fn from_state(name: &str) -> Mode {
        match name {
            "save" => Mode::Save,
            "load" => Mode::Load,
            "corner" => Mode::Corner,
            _ => Mode::Data,
        }
    }

    /// `StructureMode.LEGACY_CODEC`: the enum constant's name.
    fn legacy(self) -> &'static str {
        match self {
            Mode::Save => "SAVE",
            Mode::Load => "LOAD",
            Mode::Corner => "CORNER",
            Mode::Data => "DATA",
        }
    }

    fn from_legacy(name: &str) -> Option<Mode> {
        Some(match name {
            "SAVE" => Mode::Save,
            "LOAD" => Mode::Load,
            "CORNER" => Mode::Corner,
            "DATA" => Mode::Data,
            _ => return None,
        })
    }
}

/// `Rotation` by ordinal (none, 90, 180, -90).
pub(crate) const ROTATIONS: [&str; 4] = ["NONE", "CLOCKWISE_90", "CLOCKWISE_180", "COUNTERCLOCKWISE_90"];
/// `Mirror` by ordinal.
pub(crate) const MIRRORS: [&str; 3] = ["NONE", "LEFT_RIGHT", "FRONT_BACK"];

/// `Identifier.tryParse`: `namespace:path` (the namespace defaults to `minecraft`), `None` if it is not one.
pub(crate) fn parse_identifier(s: &str) -> Option<String> {
    let (ns, path) = match s.split_once(':') {
        Some((ns, path)) => (ns, path),
        None => ("minecraft", s),
    };
    let ns_ok = !ns.is_empty() && ns.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.'));
    let path_ok = !path.is_empty() && path.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.' | '/'));
    (ns_ok && path_ok).then(|| format!("{ns}:{path}"))
}

/// What a structure block entity keeps (`StructureBlockEntity`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Data {
    /// `structureName` (`None`: empty).
    pub name: Option<String>,
    pub author: String,
    pub metadata: String,
    pub pos: [i32; 3],
    pub size: [i32; 3],
    /// Ordinals of [`ROTATIONS`] and [`MIRRORS`].
    pub rotation: u8,
    pub mirror: u8,
    pub mode: Mode,
    pub ignore_entities: bool,
    pub strict: bool,
    pub powered: bool,
    pub show_air: bool,
    pub show_bounding_box: bool,
    pub integrity: f32,
    pub seed: i64,
}

impl Default for Data {
    fn default() -> Data {
        Data {
            name: None,
            author: String::new(),
            metadata: String::new(),
            pos: [0, 1, 0],
            size: [0, 0, 0],
            rotation: 0,
            mirror: 0,
            mode: Mode::Data,
            ignore_entities: true,
            strict: false,
            powered: false,
            show_air: false,
            show_bounding_box: true,
            integrity: 1.0,
            seed: 0,
        }
    }
}

impl Data {
    /// `StructureBlockEntity.loadAdditional`.
    pub fn load(nbt: &Tag) -> Data {
        let int = |k: &str, d: i32| nbt.get(k).and_then(Tag::as_i64).map_or(d, |v| v as i32);
        let flag = |k: &str, d: bool| nbt.get(k).and_then(Tag::as_i64).map_or(d, |v| v != 0);
        let text = |k: &str| nbt.get(k).and_then(Tag::as_str).unwrap_or("").to_owned();
        let name = text("name");
        let legacy = |k: &str, all: &[&str]| nbt.get(k).and_then(Tag::as_str).and_then(|s| all.iter().position(|a| *a == s));
        let defaults = Data::default();
        Data {
            name: if name.is_empty() { None } else { parse_identifier(&name) },
            author: text("author"),
            metadata: text("metadata"),
            pos: [int("posX", defaults.pos[0]).clamp(-48, 48), int("posY", defaults.pos[1]).clamp(-48, 48), int("posZ", defaults.pos[2]).clamp(-48, 48)],
            size: [int("sizeX", 0).clamp(0, 48), int("sizeY", 0).clamp(0, 48), int("sizeZ", 0).clamp(0, 48)],
            rotation: legacy("rotation", &ROTATIONS).unwrap_or(0) as u8,
            mirror: legacy("mirror", &MIRRORS).unwrap_or(0) as u8,
            mode: nbt.get("mode").and_then(Tag::as_str).and_then(Mode::from_legacy).unwrap_or(Mode::Data),
            ignore_entities: flag("ignoreEntities", true),
            strict: flag("strict", false),
            powered: flag("powered", false),
            show_air: flag("showair", false),
            show_bounding_box: flag("showboundingbox", true),
            integrity: nbt.get("integrity").and_then(Tag::as_f64).map_or(1.0, |v| v as f32),
            seed: nbt.get("seed").and_then(Tag::as_i64).unwrap_or(0),
        }
    }

    /// `StructureBlockEntity.saveAdditional`.
    pub fn save(&self, out: &mut Vec<(String, Tag)>) {
        let mut put = |k: &str, v: Tag| out.push((k.to_owned(), v));
        put("name", Tag::String(self.name.clone().unwrap_or_default()));
        put("author", Tag::String(self.author.clone()));
        put("metadata", Tag::String(self.metadata.clone()));
        put("posX", Tag::Int(self.pos[0]));
        put("posY", Tag::Int(self.pos[1]));
        put("posZ", Tag::Int(self.pos[2]));
        put("sizeX", Tag::Int(self.size[0]));
        put("sizeY", Tag::Int(self.size[1]));
        put("sizeZ", Tag::Int(self.size[2]));
        put("rotation", Tag::String(ROTATIONS[self.rotation as usize].into()));
        put("mirror", Tag::String(MIRRORS[self.mirror as usize].into()));
        put("mode", Tag::String(self.mode.legacy().into()));
        put("ignoreEntities", Tag::Byte(self.ignore_entities as i8));
        put("strict", Tag::Byte(self.strict as i8));
        put("powered", Tag::Byte(self.powered as i8));
        put("showair", Tag::Byte(self.show_air as i8));
        put("showboundingbox", Tag::Byte(self.show_bounding_box as i8));
        put("integrity", Tag::Float(self.integrity));
        put("seed", Tag::Long(self.seed));
    }
}

/// What a jigsaw block entity keeps (`JigsawBlockEntity`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Jigsaw {
    pub name: String,
    pub target: String,
    pub pool: String,
    pub final_state: String,
    /// `JointType.ROLLABLE` (else `ALIGNED`); `None` until the block's state says (`getDefaultJointType`).
    pub rollable: Option<bool>,
    pub placement_priority: i32,
    pub selection_priority: i32,
}

impl Default for Jigsaw {
    fn default() -> Jigsaw {
        Jigsaw {
            name: "minecraft:empty".into(),
            target: "minecraft:empty".into(),
            pool: "minecraft:empty".into(),
            final_state: "minecraft:air".into(),
            rollable: None,
            placement_priority: 0,
            selection_priority: 0,
        }
    }
}

impl Jigsaw {
    /// `JigsawBlockEntity.loadAdditional`.
    pub fn load(nbt: &Tag) -> Jigsaw {
        let id = |k: &str| nbt.get(k).and_then(Tag::as_str).and_then(parse_identifier);
        let d = Jigsaw::default();
        Jigsaw {
            name: id("name").unwrap_or(d.name),
            target: id("target").unwrap_or(d.target),
            pool: id("pool").unwrap_or(d.pool),
            final_state: nbt.get("final_state").and_then(Tag::as_str).map_or(d.final_state, str::to_owned),
            rollable: match nbt.get("joint").and_then(Tag::as_str) {
                Some("rollable") => Some(true),
                Some("aligned") => Some(false),
                _ => None,
            },
            placement_priority: nbt.get("placement_priority").and_then(Tag::as_i64).map_or(0, |v| v as i32),
            selection_priority: nbt.get("selection_priority").and_then(Tag::as_i64).map_or(0, |v| v as i32),
        }
    }

    /// `JigsawBlockEntity.saveAdditional`; a joint not decided yet is saved as the rollable default.
    pub fn save(&self, out: &mut Vec<(String, Tag)>) {
        out.push(("name".into(), Tag::String(self.name.clone())));
        out.push(("target".into(), Tag::String(self.target.clone())));
        out.push(("pool".into(), Tag::String(self.pool.clone())));
        out.push(("final_state".into(), Tag::String(self.final_state.clone())));
        out.push(("joint".into(), Tag::String(if self.rollable.unwrap_or(true) { "rollable" } else { "aligned" }.into())));
        out.push(("placement_priority".into(), Tag::Int(self.placement_priority)));
        out.push(("selection_priority".into(), Tag::Int(self.selection_priority)));
    }
}

// ---- the block entities in the regions ----------------------------------------------------------------------------------

use crate::blocks::RegionLevel;
use crate::container::BeKind;
use crate::{ConnId, DimId, Sim};
use kiln_blocks::{BlockPos, Level, state};
use kiln_world::Blocks as _;
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_worldgen::structure::template::{BlockInfo, PlaceSettings, Template};
use std::collections::HashMap;
use std::sync::Arc;

fn data<'a>(l: &'a mut RegionLevel<'_>, bp: BlockPos) -> Option<&'a mut Data> {
    l.blocks.containers.get_mut(bp).and_then(|c| c.structure.as_deref_mut())
}

/// A new block entity takes what its block says (`StructureBlockEntity`'s constructor: the block's mode; `JigsawBlockEntity`'s:
/// rollable).
pub(crate) fn created(level: &mut RegionLevel, pos: BlockPos) {
    let s = level.block(pos);
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    match c.kind {
        BeKind::StructureBlock => {
            if let Some(d) = c.structure.as_deref_mut() {
                d.mode = Mode::from_state(state::get(s, "mode").unwrap_or("load"));
            }
        }
        BeKind::Jigsaw => {
            if let Some(j) = c.jigsaw.as_deref_mut() {
                j.rollable = Some(true);
            }
        }
        _ => return,
    }
    c.mark_changed();
    crate::container::open::sync_chunk_copy(level, pos);
}

/// `loadAdditional` has run on the block entity at `pos`: a structure block's block follows the mode it loaded
/// (`updateBlockState`), a jigsaw block's joint that the data left out is the block's default
/// (`StructureTemplate.getDefaultJointType`).
pub(crate) fn loaded(level: &mut RegionLevel, pos: BlockPos) {
    let s = level.block(pos);
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    match c.kind {
        BeKind::StructureBlock => {
            let Some(mode) = c.structure.as_deref().map(|d| d.mode) else { return };
            if logic::is_instance(s, C::StructureBlock) {
                let ns = state::set(s, "mode", mode.state_name());
                if ns != s {
                    kiln_blocks::set_block(level, pos, ns, kiln_blocks::flags::CLIENTS);
                }
            }
        }
        BeKind::Jigsaw => {
            if let Some(j) = c.jigsaw.as_deref_mut()
                && j.rollable.is_none()
            {
                let front = state::get(s, "orientation").unwrap_or("north_up").split('_').next().unwrap_or("north");
                j.rollable = Some(matches!(front, "up" | "down"));
            }
        }
        _ => return,
    }
    crate::container::open::sync_chunk_copy(level, pos);
}

/// `StructureBlock.neighborChanged`: the power changes; a block that was not powered runs its mode (in the serial phase).
pub(crate) fn powered_changed(level: &mut RegionLevel, pos: BlockPos, powered: bool) {
    let Some(d) = data(level, pos) else { return };
    if powered && !d.powered {
        d.powered = true;
        level.blocks.structure_triggers.push(pos);
    } else if !powered && d.powered {
        d.powered = false;
    }
}

/// `StructureBlock.setPlacedBy`: the placer is the author.
pub(crate) fn placed_by(level: &mut RegionLevel, pos: BlockPos, author: &str) {
    if let Some(d) = data(level, pos) {
        d.author = author.to_owned();
        if let Some(c) = level.blocks.containers.get_mut(pos) {
            c.mark_changed();
        }
        crate::container::open::sync_chunk_copy(level, pos);
    }
}

// ---- the templates --------------------------------------------------------------------------------------------------------

/// A template of the manager with the author it was saved under (`StructureTemplate.author`, "?" until a structure block
/// saves it).
pub(crate) struct Stored {
    pub template: Arc<Template>,
    pub author: String,
}

/// `StructureTemplateManager`: the templates asked for so far (an absent one is remembered too), and, for a server that
/// has no world directory, the files it would have written.
#[derive(Default)]
pub(crate) struct Templates {
    map: HashMap<String, Option<Arc<Stored>>>,
    disk: HashMap<String, Vec<u8>>,
}

impl Templates {
    pub fn cached(&self, id: &str) -> Option<Option<Arc<Stored>>> {
        self.map.get(id).cloned()
    }

    pub fn insert(&mut self, id: &str, stored: Option<Arc<Stored>>) {
        self.map.insert(id.to_owned(), stored);
    }

    /// `StructureTemplateManager.remove`.
    pub fn remove(&mut self, id: &str) {
        self.map.remove(id);
    }
}

/// The path of a generated template inside a world directory (`createAndValidatePathToGeneratedStructure`).
fn generated_path(world: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
    let (ns, path) = id.split_once(':')?;
    if path.split('/').any(|s| s == ".." || s == "." || s.is_empty()) {
        return None;
    }
    Some(world.join("generated").join(ns).join("structures").join(format!("{path}.nbt")))
}

/// The generator whose geometry a level without one of its own places templates over.
fn fallback_generator(dim: DimId) -> Option<&'static kiln_worldgen::Generator> {
    use kiln_worldgen::generator::BiomeSourceKind;
    static GENERATORS: [std::sync::OnceLock<Option<kiln_worldgen::Generator>>; 3] = [std::sync::OnceLock::new(), std::sync::OnceLock::new(), std::sync::OnceLock::new()];
    GENERATORS[dim]
        .get_or_init(|| {
            let pack = kiln_worldgen::Datapack::load(&crate::datapack_dir(None)).ok()?;
            match dim {
                crate::NETHER_ID => kiln_worldgen::Generator::for_dimension(&pack, "minecraft:nether", BiomeSourceKind::MultiNoise("minecraft:nether"), "minecraft:the_nether", 0).ok(),
                crate::END_ID => kiln_worldgen::Generator::for_dimension(&pack, "minecraft:end", BiomeSourceKind::TheEnd, "minecraft:the_end", 0).ok(),
                _ => kiln_worldgen::Generator::new(&pack, "minecraft:overworld", "minecraft:overworld", 0).ok(),
            }
        })
        .as_ref()
}

fn mirror_of(m: u8) -> kiln_worldgen::structure::transform::Mirror {
    use kiln_worldgen::structure::transform::Mirror;
    [Mirror::None, Mirror::LeftRight, Mirror::FrontBack][m as usize % 3]
}

fn rotation_of(r: u8) -> kiln_worldgen::structure::transform::Rotation {
    kiln_worldgen::structure::transform::Rotation::ALL[r as usize % 4]
}

impl Sim {
    /// `StructureTemplateManager.get`: the cached template, else a file the world generated, else the packs' and the game's.
    pub(crate) fn template_get(&mut self, id: &str) -> Option<Arc<Stored>> {
        if let Some(cached) = self.world.templates.cached(id) {
            return cached;
        }
        let from_file = match self.storage.as_ref() {
            Some(s) => generated_path(&s.dir, id).and_then(|p| kiln_worldgen::structure::template::read_template_file(&p)),
            None => self.world.templates.disk.get(id).and_then(|b| kiln_worldgen::structure::template::read_template_bytes(b)),
        };
        let found = match from_file {
            Some(t) => Some(Arc::new(Stored { template: Arc::new(t), author: "?".into() })),
            None => self.find_template(id).map(|t| Arc::new(Stored { template: t, author: "?".into() })),
        };
        self.world.templates.insert(id, found.clone());
        found
    }

    /// `StructureTemplateManager.save`: the template written under the world's generated structures.
    fn template_save(&mut self, id: &str) -> bool {
        let Some(Some(stored)) = self.world.templates.cached(id) else { return false };
        let bytes = stored.template.to_file(kiln_storage::anvil::DATA_VERSION as i32);
        match self.storage.as_ref().map(|s| s.dir.clone()) {
            Some(dir) => {
                let Some(file) = generated_path(&dir, id) else { return false };
                if let Some(parent) = file.parent()
                    && std::fs::create_dir_all(parent).is_err()
                {
                    return false;
                }
                std::fs::write(&file, bytes).is_ok()
            }
            None => {
                self.world.templates.disk.insert(id.to_owned(), bytes);
                true
            }
        }
    }

    /// The saved templates, for tests: the NBT of the template `id` if the manager has it.
    pub(crate) fn template_nbt(&self, id: &str) -> Option<Tag> {
        let stored = self.world.templates.cached(id)??;
        Some(stored.template.save(kiln_storage::anvil::DATA_VERSION as i32))
    }

    fn structure_data<R>(&mut self, dim: DimId, pos: [i32; 3], f: impl FnOnce(&mut Data) -> R) -> Option<R> {
        self.with_level_in(dim, pos, |l| data(l, BlockPos::new(pos[0], pos[1], pos[2])).map(f)).flatten()
    }

    /// `StructureBlockEntity.setChanged` and `Level.sendBlockUpdated`: the clients see the block again with its data.
    fn structure_sent(&mut self, dim: DimId, pos: [i32; 3]) {
        self.with_level_in(dim, pos, |l| {
            let bp = BlockPos::new(pos[0], pos[1], pos[2]);
            if let Some(c) = l.blocks.containers.get_mut(bp) {
                c.mark_changed();
            }
            crate::container::open::sync_chunk_copy(l, bp);
            l.out.changed.push(pos);
        });
    }

    /// `ServerGamePacketListenerImpl.handleSetStructureBlock`.
    pub(crate) fn set_structure_block(&mut self, conn: ConnId, u: &kiln_proto::packets::serverbound::StructureBlockUpdate) {
        use kiln_proto::packets::serverbound as sb;
        let Some(p) = self.players.get(&conn) else { return };
        if !p.can_use_gamemaster_blocks() {
            return;
        }
        let (dim, pos) = (p.dim, u.pos);
        if self.structure_data(dim, pos, |_| ()).is_none() {
            return;
        }
        let mode = match u.mode {
            sb::StructureMode::Save => Mode::Save,
            sb::StructureMode::Load => Mode::Load,
            sb::StructureMode::Corner => Mode::Corner,
            sb::StructureMode::Data => Mode::Data,
        };
        let name = if u.name.is_empty() { None } else { parse_identifier(&u.name) };
        self.with_level_in(dim, pos, |l| {
            let bp = BlockPos::new(pos[0], pos[1], pos[2]);
            let Some(d) = data(l, bp) else { return };
            // `setMode`: the block follows.
            d.mode = mode;
            d.name = name.clone();
            d.pos = u.offset.map(i32::from);
            d.size = u.size.map(i32::from);
            d.mirror = match u.mirror {
                sb::Mirror::None => 0,
                sb::Mirror::LeftRight => 1,
                sb::Mirror::FrontBack => 2,
            };
            d.rotation = match u.rotation {
                sb::Rotation::None => 0,
                sb::Rotation::Clockwise90 => 1,
                sb::Rotation::Clockwise180 => 2,
                sb::Rotation::CounterClockwise90 => 3,
            };
            d.metadata = u.metadata.clone();
            d.ignore_entities = u.ignore_entities;
            d.strict = u.strict;
            d.show_air = u.show_air;
            d.show_bounding_box = u.show_bounding_box;
            d.integrity = u.integrity;
            d.seed = u.seed;
            let s = l.block(bp);
            if logic::is_instance(s, C::StructureBlock) {
                let ns = state::set(s, "mode", mode.state_name());
                kiln_blocks::set_block(l, bp, ns, 0);
            }
        });
        let message = if let Some(name) = name {
            let text = |key: &str| kiln_command::Text::translate(key, vec![kiln_command::text::Arg::from(name.clone())]);
            match u.update_type {
                sb::StructureUpdateType::SaveArea => {
                    Some(if self.structure_save(dim, pos, true) { text("structure_block.save_success") } else { text("structure_block.save_failure") })
                }
                sb::StructureUpdateType::LoadArea => Some(if !self.structure_loadable(dim, pos) {
                    text("structure_block.load_not_found")
                } else if self.structure_place_if_same_size(dim, pos) {
                    text("structure_block.load_success")
                } else {
                    text("structure_block.load_prepare")
                }),
                sb::StructureUpdateType::ScanArea => Some(if self.structure_detect_size(dim, pos) {
                    text("structure_block.size_success")
                } else {
                    kiln_command::Text::translate("structure_block.size_failure", Vec::new())
                }),
                sb::StructureUpdateType::UpdateData => None,
            }
        } else {
            Some(kiln_command::Text::translate("structure_block.invalid_structure_name", vec![kiln_command::text::Arg::from(u.name.clone())]))
        };
        if let (Some(m), Some(p)) = (message, self.players.get_mut(&conn)) {
            p.send(kiln_proto::packets::system_chat(m.to_nbt(), false));
        }
        self.structure_sent(dim, pos);
    }

    /// `ServerGamePacketListenerImpl.handleSetJigsawBlock`.
    pub(crate) fn set_jigsaw_block(&mut self, conn: ConnId, u: &kiln_proto::packets::serverbound::JigsawBlockUpdate) {
        let Some(p) = self.players.get(&conn) else { return };
        if !p.can_use_gamemaster_blocks() {
            return;
        }
        let (dim, pos) = (p.dim, u.pos);
        let found = self
            .with_level_in(dim, pos, |l| {
                let bp = BlockPos::new(pos[0], pos[1], pos[2]);
                let Some(j) = l.blocks.containers.get_mut(bp).and_then(|c| c.jigsaw.as_deref_mut()) else { return false };
                j.name = u.name.clone();
                j.target = u.target.clone();
                j.pool = u.pool.clone();
                j.final_state = u.final_state.clone();
                j.rollable = Some(u.rollable);
                j.placement_priority = u.placement_priority;
                j.selection_priority = u.selection_priority;
                true
            })
            .unwrap_or(false);
        if found {
            self.structure_sent(dim, pos);
        }
    }

    /// `ServerGamePacketListenerImpl.handleJigsawGenerate`: `JigsawBlockEntity.generate`.
    pub(crate) fn jigsaw_generate(&mut self, conn: ConnId, pos: [i32; 3], levels: i32, _keep_jigsaws: bool) {
        let Some(p) = self.players.get(&conn) else { return };
        if !p.can_use_gamemaster_blocks() {
            return;
        }
        let dim = p.dim;
        let found = self
            .with_level_in(dim, pos, |l| {
                let bp = BlockPos::new(pos[0], pos[1], pos[2]);
                let s = l.block(bp);
                let j = l.blocks.containers.get(bp).and_then(|c| c.jigsaw.as_deref())?;
                let front = state::get(s, "orientation").unwrap_or("north_up").split('_').next().unwrap_or("north").to_owned();
                Some((j.pool.clone(), j.target.clone(), front))
            })
            .flatten();
        let Some((pool, target, front)) = found else { return };
        let d = match front.as_str() {
            "down" => [0, -1, 0],
            "up" => [0, 1, 0],
            "north" => [0, 0, -1],
            "south" => [0, 0, 1],
            "west" => [-1, 0, 0],
            _ => [1, 0, 0],
        };
        let at = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
        let _ = self.place_generated_jigsaw(dim, &pool, &target, levels, at);
    }

    /// The serial phase of the structure blocks that were powered (`StructureBlock.trigger`).
    pub(crate) fn run_structure_triggers(&mut self) {
        let mut due: Vec<(DimId, [i32; 3])> = Vec::new();
        for dim in 0..self.dims.len() {
            for region in self.dims[dim].regions.iter_mut() {
                for p in std::mem::take(&mut region.part_mut().1.structure_triggers) {
                    due.push((dim, [p.x, p.y, p.z]));
                }
            }
        }
        due.sort_unstable();
        due.dedup();
        for (dim, pos) in due {
            match self.structure_data(dim, pos, |d| d.mode) {
                Some(Mode::Save) => {
                    self.structure_save(dim, pos, false);
                }
                Some(Mode::Load) => self.structure_place(dim, pos),
                Some(Mode::Corner) => {
                    if let Some(Some(name)) = self.structure_data(dim, pos, |d| d.name.clone()) {
                        self.world.templates.remove(&name);
                    }
                }
                _ => {}
            }
        }
    }

    /// `StructureBlockEntity.saveStructure(writeToDisk)`: the area copied into the template of the name.
    fn structure_save(&mut self, dim: DimId, pos: [i32; 3], write: bool) -> bool {
        let Some((mode, name, offset, size, ignore_entities, author)) = self.structure_data(dim, pos, |d| (d.mode, d.name.clone(), d.pos, d.size, d.ignore_entities, d.author.clone())) else {
            return false;
        };
        // (`saveStructure()` is for save mode; the redstone trigger comes here in save mode too.)
        let Some(name) = name.filter(|_| mode == Mode::Save) else { return false };
        let origin = [pos[0] + offset[0], pos[1] + offset[1], pos[2] + offset[2]];
        let old = self.template_get(&name);
        let template = if size.iter().all(|&s| s >= 1) {
            Arc::new(self.fill_from_world(dim, origin, size, !ignore_entities))
        } else {
            // (`StructureTemplate.fillFromWorld` does nothing for an area without volume: a new template stays empty.)
            old.map_or_else(|| Arc::new(Template::default()), |o| o.template.clone())
        };
        self.world.templates.insert(&name, Some(Arc::new(Stored { template, author })));
        !write || self.template_save(&name)
    }

    /// `StructureTemplate.fillFromWorld`: the blocks of the area (structure voids left out) with their block entities.
    fn fill_from_world(&self, dim: DimId, origin: [i32; 3], size: [i32; 3], _with_entities: bool) -> Template {
        use kiln_worldgen::pos::BlockPos as WPos;
        let mut infos: Vec<BlockInfo> = Vec::new();
        for y in 0..size[1] {
            for z in 0..size[2] {
                for x in 0..size[0] {
                    let at = [origin[0] + x, origin[1] + y, origin[2] + z];
                    let s = self.block_at_in(dim, at).unwrap_or(kiln_data::blocks::default_state::AIR);
                    if kiln_data::blocks_types::block_of(s).name == "minecraft:structure_void" {
                        continue;
                    }
                    let nbt = self.block_entity_for_template(dim, at).map(Arc::new);
                    infos.push(BlockInfo { pos: WPos::new(x, y, z), state: s, nbt });
                }
            }
        }
        Template::from_world(size, infos, Vec::new())
    }

    /// `BlockEntity.saveWithId` of the block entity at `at`, as the template keeps it.
    fn block_entity_for_template(&self, dim: DimId, at: [i32; 3]) -> Option<Tag> {
        let chunk_pos = kiln_world::ChunkPos::of_block(at[0], at[2]);
        let chunk = self.dims[dim].regions.chunk(chunk_pos)?;
        let be = chunk.block_entity((at[0] & 15) as usize, at[1], (at[2] & 15) as usize)?;
        let Tag::Compound(mut fields) = be.saved(at) else { return None };
        fields.retain(|(k, _)| !matches!(k.as_str(), "x" | "y" | "z"));
        // A live container's state is newer than the chunk's copy.
        if let Some(Tag::Compound(live)) = self.block_entity_live(dim, at[0], at[1], at[2]) {
            for (k, v) in live {
                fields.retain(|(ok, _)| *ok != k);
                fields.push((k, v));
            }
        }
        if !fields.iter().any(|(k, _)| k == "components") {
            fields.push(("components".into(), Tag::Compound(Vec::new())));
        }
        Some(Tag::Compound(fields))
    }

    /// `StructureBlockEntity.isStructureLoadable`.
    fn structure_loadable(&mut self, dim: DimId, pos: [i32; 3]) -> bool {
        match self.structure_data(dim, pos, |d| (d.mode, d.name.clone())) {
            Some((Mode::Load, Some(name))) => self.template_get(&name).is_some(),
            _ => false,
        }
    }

    /// `StructureBlockEntity.placeStructureIfSameSize`: placed when the area has the template's size, else the block only
    /// learns the size and the author.
    fn structure_place_if_same_size(&mut self, dim: DimId, pos: [i32; 3]) -> bool {
        let Some((mode, name, size)) = self.structure_data(dim, pos, |d| (d.mode, d.name.clone(), d.size)) else { return false };
        let Some(name) = name.filter(|_| mode == Mode::Load) else { return false };
        let Some(stored) = self.template_get(&name) else { return false };
        if stored.template.size == size {
            self.structure_place_stored(dim, pos, &stored);
            true
        } else {
            self.structure_learn(dim, pos, &stored);
            false
        }
    }

    /// `StructureBlockEntity.placeStructure(level)`.
    fn structure_place(&mut self, dim: DimId, pos: [i32; 3]) {
        let Some(Some(name)) = self.structure_data(dim, pos, |d| d.name.clone()) else { return };
        if let Some(stored) = self.template_get(&name) {
            self.structure_place_stored(dim, pos, &stored);
        }
    }

    /// `StructureBlockEntity.loadStructureInfo(template)`.
    fn structure_learn(&mut self, dim: DimId, pos: [i32; 3], stored: &Stored) {
        let (author, size) = (stored.author.clone(), stored.template.size);
        self.structure_data(dim, pos, |d| {
            d.author = author;
            d.size = size;
        });
    }

    /// `StructureBlockEntity.placeStructure(level, template)`.
    fn structure_place_stored(&mut self, dim: DimId, pos: [i32; 3], stored: &Stored) {
        use kiln_worldgen::pos::BlockPos as WPos;
        use kiln_worldgen::structure::processor::Processor;
        self.structure_learn(dim, pos, stored);
        let Some(d) = self.structure_data(dim, pos, |d| d.clone()) else { return };
        let at = WPos::new(pos[0] + d.pos[0], pos[1] + d.pos[1], pos[2] + d.pos[2]);
        let (mirror, rotation) = (mirror_of(d.mirror), rotation_of(d.rotation));
        let rot = Processor::BlockRot { rottable: None, integrity: d.integrity.clamp(0.0, 1.0) };
        let seed = if d.seed == 0 { self.game_time as i64 + 1 } else { d.seed };
        let mut settings = PlaceSettings::default();
        settings.mirror = mirror;
        settings.rotation = rotation;
        settings.ignore_entities = d.ignore_entities;
        settings.known_shape = d.strict;
        if d.integrity < 1.0 {
            settings.processors.push(&rot);
            settings.random = Some(kiln_worldgen::random::WorldgenRandom::legacy(seed));
        }
        let mut random = kiln_worldgen::random::WorldgenRandom::legacy(seed);
        let flags = 2 | if d.strict { 816 } else { 0 };
        let b = kiln_worldgen::structure::template::bounding_box(at, rotation, WPos::new(0, 0, 0), mirror, stored.template.size);
        let (cx, cz) = ((at.x >> 4), (at.z >> 4));
        let reach = |lo: i32, hi: i32, c: i32| (c - (lo >> 4)).abs().max(((hi >> 4) - c).abs());
        let radius = (reach(b.min_x, b.max_x, cx).max(reach(b.min_z, b.max_z, cz)) + 1).min(10);
        let template = stored.template.clone();
        self.with_structure_region(dim, (cx, cz), radius, flags as u32, |r| {
            template.place_in_world(r, at, at, &mut settings, &mut random, flags);
        });
    }

    /// Runs `f` over the blocks around `center` as worldgen sees them and applies what it changes.
    fn with_structure_region<R>(&mut self, dim: DimId, center: (i32, i32), radius: i32, flags: u32, f: impl FnOnce(&mut kiln_worldgen::region::Region) -> R) -> Option<R> {
        match self.world.pipelines.get(dim).cloned().flatten() {
            Some(pipeline) => {
                let world = pipeline.world().clone();
                self.with_live_region_in(dim, center, radius, flags, &world.generator, f)
            }
            None => {
                let g = fallback_generator(dim)?;
                self.with_live_region_in(dim, center, radius, flags, g, f)
            }
        }
    }

    /// `StructureBlockEntity.detectSize`: the corners of the same name enclose the area.
    fn structure_detect_size(&mut self, dim: DimId, pos: [i32; 3]) -> bool {
        let Some((mode, name)) = self.structure_data(dim, pos, |d| (d.mode, d.name.clone())) else { return false };
        if mode != Mode::Save {
            return false;
        }
        let mut corners: Vec<[i32; 3]> = Vec::new();
        for region in self.dims[dim].regions.iter() {
            for (p, c) in region.part().1.containers.map.iter() {
                let Some(d) = c.structure.as_deref() else { continue };
                let inside = (p.x - pos[0]).abs() <= 80 && (p.z - pos[2]).abs() <= 80;
                if inside && d.mode == Mode::Corner && d.name == name {
                    corners.push([p.x, p.y, p.z]);
                }
            }
        }
        let mut bb: Option<([i32; 3], [i32; 3])> = None;
        for c in &corners {
            bb = Some(match bb {
                None => (*c, *c),
                Some((lo, hi)) => ([lo[0].min(c[0]), lo[1].min(c[1]), lo[2].min(c[2])], [hi[0].max(c[0]), hi[1].max(c[1]), hi[2].max(c[2])]),
            });
        }
        let Some((mut lo, mut hi)) = bb else { return false };
        // One corner: the box reaches the structure block itself.
        if corners.len() == 1 {
            for i in 0..3 {
                lo[i] = lo[i].min(pos[i]);
                hi[i] = hi[i].max(pos[i]);
            }
        }
        let (dx, dy, dz) = (hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]);
        if dx > 1 && dy > 1 && dz > 1 {
            self.structure_data(dim, pos, |d| {
                d.pos = [lo[0] - pos[0] + 1, lo[1] - pos[1] + 1, lo[2] - pos[2] + 1];
                d.size = [dx - 1, dy - 1, dz - 1];
            });
            true
        } else {
            false
        }
    }
}
