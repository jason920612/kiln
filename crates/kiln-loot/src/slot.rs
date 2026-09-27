//! Slot sources (`SlotSource`, datapack registry `slot_source/`), read by `slots` entries.

use crate::context::{EntityTarget, SlotOwner};
use crate::data::Kind;
use crate::eval::Eval;
use crate::function::ContainerKind;
use crate::json::Json;
use crate::parse::{PResult, ParseError, Parser, Ref, fail, ident, int, opt_or, req, string};
use crate::predicate::{self, item_predicate};
use crate::stack;
use kiln_item::ItemStack;
use kiln_item::component::ItemPredicate;

#[derive(Debug, Clone)]
pub enum SlotSource {
    Group(Vec<Ref<SlotSource>>),
    Filtered { source: Ref<SlotSource>, filter: ItemPredicate },
    Limit { source: Ref<SlotSource>, limit: usize },
    /// `slot_range`: a named range (`container.*`, `armor.head`...) of an owner's slots.
    Range { owner: SlotOwner, slots: String },
    Contents { source: Ref<SlotSource>, component: ContainerKind },
    Empty,
}

impl SlotSource {
    /// `SlotSources.CODEC`: a reference, a typed object, or a list (a group).
    pub fn parse_ref(p: &Parser, j: &Json) -> PResult<Ref<SlotSource>> {
        match j {
            Json::Str(_) => p.holder(j, Kind::SlotSource, SlotSource::parse),
            _ => SlotSource::parse(p, j).map(Ref::direct),
        }
    }

    pub fn parse(p: &Parser, j: &Json) -> PResult<SlotSource> {
        if j.as_array().is_some() {
            return Ok(SlotSource::Group(p.holder_list(j, Kind::SlotSource, SlotSource::parse_typed)?));
        }
        SlotSource::parse_typed(p, j)
    }

    fn parse_typed(p: &Parser, j: &Json) -> PResult<SlotSource> {
        let ty = req(j, "type", ident)?;
        let inner = || req(j, "slot_source", |v| SlotSource::parse_ref(p, v));
        Ok(match ty.as_str() {
            "minecraft:group" => SlotSource::Group(req(j, "terms", |v| p.holder_list(v, Kind::SlotSource, SlotSource::parse_typed))?),
            "minecraft:filtered" => SlotSource::Filtered { source: inner()?, filter: req(j, "item_filter", item_predicate)? },
            "minecraft:limit_slots" => SlotSource::Limit {
                source: inner()?,
                limit: req(j, "limit", |v| {
                    let n = int(v)?;
                    if n <= 0 {
                        return fail(format!("value must be positive: {n}"));
                    }
                    Ok(n as usize)
                })?,
            },
            "minecraft:slot_range" => SlotSource::Range {
                owner: opt_or(j, "source", SlotOwner::Container, |v| {
                    let s = string(v)?;
                    match s.as_str() {
                        "container" => Ok(SlotOwner::Container),
                        "block_entity" => Ok(SlotOwner::BlockEntity),
                        _ => EntityTarget::by_name(&s)
                            .map(SlotOwner::Entity)
                            .ok_or_else(|| ParseError::new(format!("unknown slot source {s:?}"))),
                    }
                })?,
                slots: req(j, "slots", string)?,
            },
            "minecraft:contents" => SlotSource::Contents {
                source: inner()?,
                component: req(j, "component", |v| match ident(v)?.as_str() {
                    "minecraft:container" => Ok(ContainerKind::Container),
                    "minecraft:bundle_contents" => Ok(ContainerKind::BundleContents),
                    "minecraft:charged_projectiles" => Ok(ContainerKind::ChargedProjectiles),
                    other => fail(format!("unknown container component {other}")),
                })?,
            },
            "minecraft:empty" => SlotSource::Empty,
            other => return fail(format!("unknown slot source type {other}")),
        })
    }
}

impl Eval<'_> {
    /// `SlotSource.provide(context).itemCopies()`.
    pub fn slot_items(&mut self, s: &Ref<SlotSource>) -> Vec<ItemStack> {
        let data = self.data;
        let s: &SlotSource = match s {
            Ref::Direct(v) => v,
            Ref::Named(i) => match data.slot_sources.get(*i) {
                Some(v) => v,
                None => return Vec::new(),
            },
        };
        match s {
            SlotSource::Group(terms) => terms.iter().flat_map(|t| self.slot_items(t)).collect(),
            SlotSource::Filtered { source, filter } => {
                let items = self.slot_items(source);
                items.into_iter().filter(|i| predicate::item_matches(&data.tags, filter, i)).collect()
            }
            SlotSource::Limit { source, limit } => {
                let mut items = self.slot_items(source);
                items.truncate(*limit);
                items
            }
            SlotSource::Range { owner, slots } => {
                let present = match owner {
                    SlotOwner::Entity(t) => self.ctx.has_entity(*t),
                    SlotOwner::BlockEntity => self.ctx.has_block_entity(),
                    SlotOwner::Container => true,
                };
                if present { self.ctx.slot_items(*owner, slots) } else { Vec::new() }
            }
            SlotSource::Contents { source, component } => {
                let items = self.slot_items(source);
                items
                    .iter()
                    .flat_map(|i| stack::container_contents(i, *component).unwrap_or_default())
                    .collect()
            }
            SlotSource::Empty => Vec::new(),
        }
    }
}
