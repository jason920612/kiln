//! Trees: `tree` (trunk, foliage and root placers, feature sizes, decorators) and `fallen_tree`.

mod decorator;
mod fallen;
mod foliage;
mod jset;
mod root;
mod shape;
mod tree;
mod trunk;

use crate::Error;
use crate::feature::Features;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;

/// The feature types of this family.
#[derive(Debug)]
pub enum Kind {
    Tree(Box<tree::Tree>),
    FallenTree(Box<fallen::FallenTree>),
}

/// Parses a feature of this family (`ty` without the `minecraft:` prefix); `None` if the
/// type is not one of them.
pub fn parse(ty: &str, json: &Json, f: &mut Features, l: &Loader) -> Option<Result<Kind, Error>> {
    Some(match ty {
        "tree" => tree::Tree::parse(json, f, l).map(|t| Kind::Tree(Box::new(t))),
        "fallen_tree" => fallen::FallenTree::parse(json, f, l).map(|t| Kind::FallenTree(Box::new(t))),
        _ => return None,
    })
}

impl Kind {
    pub fn place(&self, f: &Features, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) -> bool {
        match self {
            Kind::Tree(t) => t.place(f, r, random, p),
            Kind::FallenTree(t) => t.place(f, r, random, p),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Kind::Tree(_) => "minecraft:tree",
            Kind::FallenTree(_) => "minecraft:fallen_tree",
        }
    }

    /// Placed features this feature places (for [`Features::is_supported`]).
    pub fn nested(&self) -> Vec<usize> {
        match self {
            Kind::Tree(t) => decorator::nested(&t.decorators),
            Kind::FallenTree(t) => {
                let mut v = decorator::nested(&t.stump_decorators);
                v.extend(decorator::nested(&t.log_decorators));
                v
            }
        }
    }
}

/// Field access with parse errors naming the missing key.
fn field<'j>(json: &'j Json, key: &str) -> Result<&'j Json, Error> {
    json.get(key).ok_or_else(|| Error::Invalid(format!("missing {key}")))
}

fn int_or(json: &Json, key: &str, default: i32) -> i32 {
    json.get(key).and_then(Json::as_i32).unwrap_or(default)
}

fn float_or(json: &Json, key: &str, default: f32) -> f32 {
    json.get(key).and_then(Json::as_f32).unwrap_or(default)
}
