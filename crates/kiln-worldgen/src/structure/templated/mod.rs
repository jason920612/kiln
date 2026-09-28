//! Structures made of one or a few templates (`TemplateStructurePiece`): igloos, shipwrecks,
//! ocean ruins, ruined portals, nether fossils and end cities.

pub mod end_city;
pub mod igloo;
pub mod nether_fossil;
pub mod ocean_ruin;
pub mod ruined_portal;
pub mod shipwreck;

use super::bbox::BoundingBox;
use super::piece::PieceBase;
use super::processor::Processor;
use super::template::{LiquidSettings, PlaceSettings, Template, TemplateManager};
use super::transform::{Mirror, Rotation};
use crate::block_facts::Dir;
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use kiln_proto::nbt::Tag;
use std::sync::Arc;

/// `TemplateStructurePiece`: a template placed at a position with fixed settings.
#[derive(Clone, Debug)]
pub struct TemplatePiece {
    pub base: PieceBase,
    pub name: String,
    pub template: Arc<Template>,
    pub rotation: Rotation,
    pub mirror: Mirror,
    pub pivot: BlockPos,
    pub processors: Vec<Processor>,
    pub liquid: LiquidSettings,
    pub position: BlockPos,
}

impl TemplatePiece {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: &'static str,
        tm: &TemplateManager,
        name: &str,
        rotation: Rotation,
        mirror: Mirror,
        pivot: BlockPos,
        processors: Vec<Processor>,
        liquid: LiquidSettings,
        position: BlockPos,
    ) -> Self {
        let template = tm.get(name);
        let mut piece = Self {
            base: PieceBase::new(kind, 0, BoundingBox::new(0, 0, 0, 0, 0, 0)),
            name: name.to_string(),
            template,
            rotation,
            mirror,
            pivot,
            processors,
            liquid,
            position,
        };
        piece.base.bbox = piece.template.bounding_box(&piece.settings(), position);
        piece.base.set_orientation(Some(Dir::North));
        piece
    }

    /// The piece's `StructurePlaceSettings`.
    pub fn settings(&self) -> PlaceSettings<'_> {
        let mut s = PlaceSettings::with_rotation(self.rotation);
        s.mirror = self.mirror;
        s.pivot = self.pivot;
        s.liquid = self.liquid;
        s.processors = self.processors.iter().collect();
        s
    }

    /// The piece's bounding box with the template at `pos`.
    pub fn bbox_at(&self, pos: BlockPos) -> BoundingBox {
        self.template.bounding_box(&self.settings(), pos)
    }

    /// `TemplateStructurePiece.addAdditionalSaveData`.
    pub fn save_template(&self, tag: &mut Vec<(String, Tag)>) {
        tag.push(("TPX".into(), Tag::Int(self.position.x)));
        tag.push(("TPY".into(), Tag::Int(self.position.y)));
        tag.push(("TPZ".into(), Tag::Int(self.position.z)));
        tag.push(("Template".into(), Tag::String(self.name.clone())));
    }

    /// `TemplateStructurePiece.postProcess` with the template at `pos`: places it, then runs
    /// `marker` for each data marker (`metadata`, position) and turns jigsaw blocks into their
    /// final states.
    pub fn post_process(
        &self,
        pos: BlockPos,
        r: &mut Region,
        random: &mut WorldgenRandom,
        chunk_box: &BoundingBox,
        pivot: BlockPos,
        marker: &mut dyn FnMut(&str, BlockPos, &mut Region, &mut WorldgenRandom),
    ) {
        let mut settings = self.settings();
        settings.bbox = Some(*chunk_box);
        if !self.template.place_in_world(r, pos, pivot, &mut settings, random, 2) {
            return;
        }
        for m in self.template.filter_blocks(pos, &mut settings, "minecraft:structure_block") {
            let Some(nbt) = &m.nbt else { continue };
            if nbt.get("mode").and_then(Tag::as_str) != Some("DATA") {
                continue;
            }
            let metadata = nbt.get("metadata").and_then(Tag::as_str).unwrap_or("");
            marker(metadata, m.pos, r, random);
        }
        for j in self.template.filter_blocks(pos, &mut settings, "minecraft:jigsaw") {
            let Some(nbt) = &j.nbt else { continue };
            let text = nbt.get("final_state").and_then(Tag::as_str).unwrap_or("minecraft:air");
            let s = super::processor::parse_state_string(text).unwrap_or(crate::blocks::state::AIR);
            r.set(j.pos, s, 3);
        }
    }

    /// `StructurePlaceSettings.calculateRelativePosition(settings, pos)`.
    pub fn relative(&self, p: BlockPos) -> BlockPos {
        super::template::transform(p, self.mirror, self.rotation, self.pivot)
    }
}

/// Puts a chest's loot table and a seed from `random` on the chest block entity at `p`
/// (`RandomizableContainer.setBlockEntityLootTable` / `ChestBlockEntity.setLootTable`), if the
/// block there is a container of the kind.
pub fn set_loot(r: &mut Region, random: &mut WorldgenRandom, p: BlockPos, table: &str, chest_only: bool) {
    let s = r.get(p);
    let is_chest = crate::block_facts::is_instance(s, "ChestBlock");
    let ok = if chest_only {
        is_chest
    } else {
        is_chest
            || ["BarrelBlock", "DispenserBlock", "HopperBlock", "ShulkerBoxBlock", "CrafterBlock", "DecoratedPotBlock"]
                .iter()
                .any(|c| crate::block_facts::is_instance(s, c))
    };
    if ok {
        use kiln_javamath::random::RandomSource;
        let seed = random.next_long();
        super::piece::set_loot_table(r, p, table, seed);
    }
}
