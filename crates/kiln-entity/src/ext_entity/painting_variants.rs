//! The painting variants (`minecraft:painting_variant`): name, width and height (in blocks), in the order
//! of the registry now. A variant's size comes from its entry when a data pack defines or overrides it,
//! else from the vanilla datapack (the table below, `data/minecraft/painting_variant`); which variants
//! a new painting may take is the tag `#minecraft:painting_variant/placeable` (tag order).

use kiln_loot::Json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variant {
    pub name: &'static str,
    pub width: i32,
    pub height: i32,
}

impl Variant {
    /// `PaintingVariant.area`.
    pub fn area(&self) -> i32 {
        self.width * self.height
    }
}

/// The variants by network id.
pub fn variants() -> &'static [Variant] {
    static ALL: kiln_data::live::Cached<Vec<Variant>> = kiln_data::live::Cached::new();
    ALL.get(|| {
        let names = kiln_data::live::synced_entries("minecraft:painting_variant").unwrap_or(&[]);
        names.iter().map(|&name| from_entry(name)).collect()
    })
}

fn from_entry(name: &'static str) -> Variant {
    let packed = kiln_data::live::entry_data("minecraft:painting_variant", name).and_then(|text| Json::parse(text).ok());
    if let Some(json) = packed {
        let size = |key: &str| json.get(key).and_then(Json::as_i32).filter(|v| (1..=16).contains(v));
        if let (Some(width), Some(height)) = (size("width"), size("height")) {
            return Variant { name, width, height };
        }
    }
    match VANILLA.iter().find(|v| v.0 == name) {
        Some(&(_, width, height)) => Variant { name, width, height },
        None => Variant { name, width: 1, height: 1 },
    }
}

/// `#minecraft:painting_variant/placeable`: network ids in the tag's order.
pub fn placeable() -> &'static [i32] {
    kiln_data::live::tag("minecraft:painting_variant", "minecraft:placeable").unwrap_or(&[])
}

/// The vanilla datapack's variants: name, width, height.
const VANILLA: &[(&str, i32, i32)] = &[
    ("minecraft:alban", 1, 1),
    ("minecraft:aztec", 1, 1),
    ("minecraft:aztec2", 1, 1),
    ("minecraft:backyard", 3, 4),
    ("minecraft:baroque", 2, 2),
    ("minecraft:bomb", 1, 1),
    ("minecraft:bouquet", 3, 3),
    ("minecraft:burning_skull", 4, 4),
    ("minecraft:bust", 2, 2),
    ("minecraft:cavebird", 3, 3),
    ("minecraft:changing", 4, 2),
    ("minecraft:cotan", 3, 3),
    ("minecraft:courbet", 2, 1),
    ("minecraft:creebet", 2, 1),
    ("minecraft:dennis", 3, 3),
    ("minecraft:donkey_kong", 4, 3),
    ("minecraft:earth", 2, 2),
    ("minecraft:endboss", 3, 3),
    ("minecraft:fern", 3, 3),
    ("minecraft:fighters", 4, 2),
    ("minecraft:finding", 4, 2),
    ("minecraft:fire", 2, 2),
    ("minecraft:graham", 1, 2),
    ("minecraft:humble", 2, 2),
    ("minecraft:kebab", 1, 1),
    ("minecraft:lowmist", 4, 2),
    ("minecraft:match", 2, 2),
    ("minecraft:meditative", 1, 1),
    ("minecraft:orb", 4, 4),
    ("minecraft:owlemons", 3, 3),
    ("minecraft:passage", 4, 2),
    ("minecraft:pigscene", 4, 4),
    ("minecraft:plant", 1, 1),
    ("minecraft:pointer", 4, 4),
    ("minecraft:pond", 3, 4),
    ("minecraft:pool", 2, 1),
    ("minecraft:prairie_ride", 1, 2),
    ("minecraft:sea", 2, 1),
    ("minecraft:skeleton", 4, 3),
    ("minecraft:skull_and_roses", 2, 2),
    ("minecraft:stage", 2, 2),
    ("minecraft:sunflowers", 3, 3),
    ("minecraft:sunset", 2, 1),
    ("minecraft:tides", 3, 3),
    ("minecraft:unpacked", 4, 4),
    ("minecraft:void", 2, 2),
    ("minecraft:wanderer", 1, 2),
    ("minecraft:wasteland", 1, 1),
    ("minecraft:water", 2, 2),
    ("minecraft:wind", 2, 2),
    ("minecraft:wither", 2, 2),
];
