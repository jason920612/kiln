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
