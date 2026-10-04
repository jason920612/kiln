//! Jigsaw structures (`JigsawStructure`): villages, outposts, bastions, ancient cities, trail
//! ruins, trial chambers and camps, grown from template pools by [`placement`].

pub mod piece;
pub mod placement;
pub mod pool;

use super::kinds::Kind;
use super::{GenCtx, Stub};
use crate::Error;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::providers::{GenContext, HeightProvider};
use crate::sets::Loader;
use crate::structure::template::LiquidSettings;
use placement::AliasBinding;
use pool::Pools;
use std::cell::RefCell;
use std::sync::Arc;

/// `JigsawStructure`.
pub struct JigsawStructure {
    pub pools: Arc<Pools>,
    pub start_pool: String,
    pub start_jigsaw_name: Option<String>,
    pub max_depth: i32,
    pub start_height: HeightProvider,
    pub use_expansion_hack: bool,
    pub project_start_to_heightmap: Option<Heightmap>,
    /// (horizontal, vertical).
    pub max_distance: (i32, i32),
    pub aliases: Vec<AliasBinding>,
    /// (bottom, top); `None` is `DimensionPadding.ZERO`, which skips the height check.
    pub padding: Option<(i32, i32)>,
    pub liquid: LiquidSettings,
}

impl Kind for JigsawStructure {
    /// `JigsawStructure.findGenerationPoint`.
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        let g = GenContext { min_y: ctx.generator.gen_min_y, height: ctx.generator.gen_height, sea_level: ctx.generator.sea_level };
        let y = self.start_height.sample(&mut ctx.random, g);
        let pos = BlockPos::new(ctx.chunk.0 << 4, y, ctx.chunk.1 << 4);
        placement::add_pieces(self, ctx, pos)
    }

    fn as_jigsaw(&self) -> Option<&JigsawStructure> {
        Some(self)
    }
}

thread_local! {
    static POOLS: RefCell<Option<Arc<Pools>>> = const { RefCell::new(None) };
}

/// Keeps the datapack's template pools loaded while its structures are parsed, so every
/// jigsaw structure shares them.
pub struct LoadScope;

impl LoadScope {
    pub fn new(l: &Loader) -> Result<LoadScope, Error> {
        let pools = Arc::new(Pools::load(l)?);
        POOLS.with(|p| *p.borrow_mut() = Some(pools));
        Ok(LoadScope)
    }
}

impl Drop for LoadScope {
    fn drop(&mut self) {
        POOLS.with(|p| *p.borrow_mut() = None);
    }
}

fn pools(l: &Loader) -> Result<Arc<Pools>, Error> {
    if let Some(p) = POOLS.with(|p| p.borrow().clone()) {
        return Ok(p);
    }
    Ok(Arc::new(Pools::load(l)?))
}

fn pair(json: Option<&Json>, first: &str, second: &str) -> Option<(i32, i32)> {
    let json = json?;
    if let Some(v) = json.as_i32() {
        return Some((v, v));
    }
    Some((json.get(first).and_then(Json::as_i32)?, json.get(second).and_then(Json::as_i32)?))
}

fn parse_alias(json: &Json) -> Result<AliasBinding, Error> {
    let ty = json.get("type").and_then(Json::as_str).unwrap_or("");
    let id = |k: &str| {
        json.get(k).and_then(Json::as_str).map(crate::function::qualify).ok_or_else(|| Error::Invalid(format!("pool alias without {k}")))
    };
    let weighted = |k: &str| json.get(k).and_then(Json::as_array).ok_or_else(|| Error::Invalid(format!("pool alias without {k}")));
    let weight = |e: &Json| e.get("weight").and_then(Json::as_i32).unwrap_or(1);
    Ok(match ty.strip_prefix("minecraft:").unwrap_or(ty) {
        "direct" => AliasBinding::Direct { alias: id("alias")?, target: id("target")? },
        "random" => AliasBinding::Random {
            alias: id("alias")?,
            targets: weighted("targets")?
                .iter()
                .map(|e| {
                    let d = e.get("data").and_then(Json::as_str).ok_or_else(|| Error::Invalid("weighted entry without data".into()))?;
                    Ok((crate::function::qualify(d), weight(e)))
                })
                .collect::<Result<_, Error>>()?,
        },
        "random_group" => AliasBinding::RandomGroup(
            weighted("groups")?
                .iter()
                .map(|e| {
                    let list = e.get("data").and_then(Json::as_array).ok_or_else(|| Error::Invalid("weighted entry without data".into()))?;
                    Ok((list.iter().map(parse_alias).collect::<Result<Vec<_>, _>>()?, weight(e)))
                })
                .collect::<Result<_, Error>>()?,
        ),
        t => return Err(Error::Invalid(format!("unknown pool alias binding {t}"))),
    })
}

/// A `minecraft:jigsaw` structure's configuration.
pub fn parse(json: &Json, l: &Loader) -> Result<JigsawStructure, Error> {
    let field = |k: &str| json.get(k).ok_or_else(|| Error::Invalid(format!("jigsaw structure without {k}")));
    let heightmap = |name: &str| Heightmap::parse(name).ok_or_else(|| Error::Invalid(format!("unknown heightmap {name}")));
    Ok(JigsawStructure {
        pools: pools(l)?,
        start_pool: crate::function::qualify(field("start_pool")?.as_str().ok_or_else(|| Error::Invalid("bad start_pool".into()))?),
        start_jigsaw_name: json.get("start_jigsaw_name").and_then(Json::as_str).map(crate::function::qualify),
        max_depth: field("size")?.as_i32().ok_or_else(|| Error::Invalid("bad size".into()))?,
        start_height: HeightProvider::parse(field("start_height")?)?,
        use_expansion_hack: field("use_expansion_hack")?.as_bool().ok_or_else(|| Error::Invalid("bad use_expansion_hack".into()))?,
        project_start_to_heightmap: json.get("project_start_to_heightmap").and_then(Json::as_str).map(heightmap).transpose()?,
        max_distance: pair(Some(field("max_distance_from_center")?), "horizontal", "vertical")
            .ok_or_else(|| Error::Invalid("bad max_distance_from_center".into()))?,
        aliases: json.get("pool_aliases").and_then(Json::as_array).unwrap_or(&[]).iter().map(parse_alias).collect::<Result<_, _>>()?,
        padding: pair(json.get("dimension_padding"), "bottom", "top"),
        liquid: match json.get("liquid_settings").and_then(Json::as_str) {
            None => LiquidSettings::ApplyWaterlogging,
            Some(n) => LiquidSettings::parse(n).ok_or_else(|| Error::Invalid(format!("bad liquid settings {n}")))?,
        },
    })
}
