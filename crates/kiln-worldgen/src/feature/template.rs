//! Template features: `TemplateFeature` (`minecraft:template`) and `FossilFeature`
//! (`minecraft:fossil`).

use crate::Error;
use crate::block_facts::Dir;
use crate::blocks::{is_air, is_lava, is_water};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use crate::structure::bbox::BoundingBox;
use crate::structure::processor::{ProcessorList, ProcessorLists};
use crate::structure::template::{PlaceSettings, TemplateManager, zero_position_with_transform};
use crate::structure::transform::{Mirror, Rotation};
use kiln_javamath::random::RandomSource;
use std::sync::Arc;

/// `TemplateFeature.TemplateEntry`.
#[derive(Debug)]
struct Entry {
    template: String,
    rotations: Vec<Rotation>,
}

/// `TemplateFeature`.
#[derive(Debug)]
pub struct TemplateFeature {
    /// (entry, weight).
    templates: Vec<(Entry, i32)>,
    processors: Option<ProcessorList>,
}

/// `FossilFeature`.
#[derive(Debug)]
pub struct Fossil {
    fossils: Vec<String>,
    overlays: Vec<String>,
    fossil_processors: ProcessorList,
    overlay_processors: ProcessorList,
    max_empty_corners: i32,
}

fn rotation(name: &str) -> Result<Rotation, Error> {
    Rotation::ALL.into_iter().find(|r| r.name() == name).ok_or_else(|| Error::Invalid(format!("unknown rotation {name}")))
}

fn ids(json: &Json, key: &str) -> Result<Vec<String>, Error> {
    json.get(key)
        .and_then(Json::as_array)
        .ok_or_else(|| Error::Invalid(format!("fossil without {key}")))?
        .iter()
        .map(|v| v.as_str().map(crate::function::qualify).ok_or_else(|| Error::Invalid(format!("bad {key}"))))
        .collect()
}

impl TemplateFeature {
    pub fn parse(json: &Json, lists: &ProcessorLists, l: &Loader) -> Result<TemplateFeature, Error> {
        let templates = json
            .get("templates")
            .and_then(Json::as_array)
            .ok_or_else(|| Error::Invalid("template feature without templates".into()))?
            .iter()
            .map(|e| {
                let data = e.get("data").ok_or_else(|| Error::Invalid("weighted entry without data".into()))?;
                let template = crate::function::qualify(data.get("id").and_then(Json::as_str).unwrap_or(""));
                let rotations = match data.get("rotations").and_then(Json::as_array) {
                    Some(list) => list.iter().map(|r| rotation(r.as_str().unwrap_or(""))).collect::<Result<_, _>>()?,
                    None => Rotation::ALL.to_vec(),
                };
                Ok((Entry { template, rotations }, e.get("weight").and_then(Json::as_i32).unwrap_or(1)))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let processors = json.get("processors").map(|p| lists.get(p, l)).transpose()?;
        Ok(TemplateFeature { templates, processors })
    }

    /// `TemplateFeature.place`.
    pub fn place(&self, tm: &TemplateManager, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let total: i32 = self.templates.iter().map(|e| e.1).sum();
        let mut i = random.next_int_bounded(total);
        let entry = &self.templates.iter().find(|e| {
            i -= e.1;
            i < 0
        })
        .expect("weights sum to the total")
        .0;
        let rot = entry.rotations[random.next_int_bounded(entry.rotations.len() as i32) as usize];
        let t = tm.get(&entry.template);
        // `getRotatedOffset`: half the size along each axis, toward the rotated negative side.
        let offset = |d: Dir, size: i32| {
            let (dx, _, dz) = rot.rotate(d).offset();
            (dx * (size / 2), dz * (size / 2))
        };
        let (ax, az) = offset(Dir::West, t.size[0]);
        let (bx, bz) = offset(Dir::North, t.size[2]);
        let p = origin.offset(ax + bx, 0, az + bz);
        let mut settings = PlaceSettings::with_rotation(rot);
        if let Some(list) = &self.processors {
            settings.processors.extend(list.iter());
        }
        t.place_with_shared_random(r, p, p, &mut settings, random, 3)
    }
}

impl Fossil {
    pub fn parse(json: &Json, lists: &ProcessorLists, l: &Loader) -> Result<Fossil, Error> {
        let field = |k: &str| json.get(k).ok_or_else(|| Error::Invalid(format!("fossil without {k}")));
        Ok(Fossil {
            fossils: ids(json, "fossil_structures")?,
            overlays: ids(json, "overlay_structures")?,
            fossil_processors: lists.get(field("fossil_processors")?, l)?,
            overlay_processors: lists.get(field("overlay_processors")?, l)?,
            max_empty_corners: field("max_empty_corners_allowed")?.as_i32().ok_or_else(|| Error::Invalid("bad max_empty_corners_allowed".into()))?,
        })
    }

    /// `FossilFeature.place`.
    pub fn place(&self, tm: &TemplateManager, r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
        let rot = Rotation::ALL[random.next_int_bounded(4) as usize];
        let i = random.next_int_bounded(self.fossils.len() as i32) as usize;
        let fossil = tm.get(&self.fossils[i]);
        let overlay = tm.get(&self.overlays[i]);
        let (cx, cz) = (origin.x >> 4, origin.z >> 4);
        let area = BoundingBox::new((cx << 4) - 16, r.min_y(), (cz << 4) - 16, (cx << 4) + 15 + 16, r.max_y(), (cz << 4) + 15 + 16);
        let mut settings = PlaceSettings::with_rotation(rot);
        settings.bbox = Some(area);
        let size = fossil.size_rotated(rot);
        let start = origin.offset(-size[0] / 2, 0, -size[2] / 2);
        let mut y = origin.y;
        for dx in 0..size[0] {
            for dz in 0..size[2] {
                y = y.min(r.height_at(Heightmap::OceanFloorWg, start.x + dx, start.z + dz));
            }
        }
        let y = (y - 15 - random.next_int_bounded(10)).max(r.min_y() + 10);
        let zero = zero_position_with_transform(start.at_y(y), Mirror::None, rot, fossil.size[0], fossil.size[2]);
        let b = fossil.bounding_box(&settings, zero);
        let mut empty = 0;
        for x in [b.min_x, b.max_x] {
            for yy in [b.min_y, b.max_y] {
                for z in [b.min_z, b.max_z] {
                    let s = r.get(BlockPos::new(x, yy, z));
                    if is_air(s) || is_lava(s) || is_water(s) {
                        empty += 1;
                    }
                }
            }
        }
        if empty > self.max_empty_corners {
            return false;
        }
        settings.processors = self.fossil_processors.iter().collect();
        fossil.place_with_shared_random(r, zero, zero, &mut settings, random, 260);
        settings.processors = self.overlay_processors.iter().collect();
        overlay.place_with_shared_random(r, zero, zero, &mut settings, random, 260);
        true
    }
}

/// The template manager features use (loaded next to the datapack).
pub fn manager(l: &Loader) -> Arc<TemplateManager> {
    Arc::new(TemplateManager::near(&l.pack.root))
}
