//! Villager trades from a datapack: `trade_set/` (how many offers a profession level gets and
//! from which trades), `villager_trade/` (one trade: prices, the item given and how it is
//! modified) and the `villager_trade` tags that group them. [`Trades::offers`] is
//! `AbstractVillager.addOffersFromTradeSet` with `VillagerTrade.getOffer`.

use crate::condition::Condition;
use crate::context::{EntityTarget, LootContext};
use crate::data::LootData;
use crate::eval::Eval;
use crate::function::Function;
use crate::json::Json;
use crate::number::{FloatProvider, IntProvider};
use crate::parse::{IdSet, PResult, ParseError, Parser, Ref, fail, ident, opt, opt_or, req};
use kiln_item::trading::{ItemCost, MerchantOffer};
use kiln_item::{DataComponentPatch, Identifier, ItemStack, ItemStackTemplate, keys, registry};
use kiln_javamath::random::RandomSource;
use std::collections::HashMap;
use std::path::Path;

/// `TradeCost`: an item, a count provider and an exact component predicate.
#[derive(Debug, Clone)]
pub struct TradeCost {
    pub item: i32,
    pub count: Ref<IntProvider>,
    pub components: DataComponentPatch,
}

impl TradeCost {
    fn parse(p: &Parser, j: &Json) -> PResult<TradeCost> {
        Ok(TradeCost {
            item: req(j, "id", |v| p.id(v, registry::ITEM))?,
            count: opt_or(j, "count", Ref::direct(IntProvider::Constant(1)), |v| IntProvider::parse_ref(p, v))?,
            components: opt_or(j, "components", DataComponentPatch::new(), |v| {
                DataComponentPatch::from_value(&v.to_value()).map_err(|e| ParseError::new(e.0))
            })?,
        })
    }

    /// `toItemCost(context, extraCount)`: the rolled count plus `extra`, clamped to the item's
    /// default stack size.
    fn to_item_cost(&self, ev: &mut Eval, extra: i32) -> ItemCost {
        let max = ItemStack::new(self.item, 1).max_stack_size();
        let n = (ev.int(&self.count) + extra).clamp(0, max);
        ItemCost { item: self.item, count: n, components: self.components.clone() }
    }
}

/// `VillagerTrade`.
#[derive(Debug, Clone)]
pub struct VillagerTrade {
    pub wants: TradeCost,
    pub additional_wants: Option<TradeCost>,
    pub gives: ItemStackTemplate,
    pub max_uses: Ref<IntProvider>,
    pub xp: Ref<IntProvider>,
    pub reputation_discount: Ref<FloatProvider>,
    pub merchant_predicate: Option<Ref<Condition>>,
    pub given_item_modifier: Option<Ref<Function>>,
    pub double_trade_price_enchantments: Option<IdSet>,
}

impl VillagerTrade {
    pub fn parse(p: &Parser, j: &Json) -> PResult<VillagerTrade> {
        Ok(VillagerTrade {
            wants: req(j, "wants", |v| TradeCost::parse(p, v))?,
            additional_wants: opt(j, "additional_wants", |v| TradeCost::parse(p, v))?,
            gives: req(j, "gives", |v| ItemStackTemplate::from_value(&v.to_value()).map_err(|e| ParseError::new(e.0)))?,
            max_uses: opt_or(j, "max_uses", Ref::direct(IntProvider::Constant(4)), |v| IntProvider::parse_ref(p, v))?,
            xp: opt_or(j, "xp", Ref::direct(IntProvider::Constant(1)), |v| IntProvider::parse_ref(p, v))?,
            reputation_discount: opt_or(j, "reputation_discount", Ref::direct(FloatProvider::Constant(0.0)), |v| FloatProvider::parse_ref(p, v))?,
            merchant_predicate: opt(j, "merchant_predicate", |v| Condition::parse_ref(p, v))?,
            given_item_modifier: opt(j, "given_item_modifier", |v| Function::parse_ref(p, v))?,
            double_trade_price_enchantments: opt(j, "double_trade_price_enchantments", |v| p.id_set(v, registry::ENCHANTMENT))?,
        })
    }

    /// `getOffer`: `None` when the merchant does not qualify or the item did not come out.
    pub fn offer(&self, ev: &mut Eval) -> Option<MerchantOffer> {
        if let Some(c) = &self.merchant_predicate
            && !ev.test(c)
        {
            return None;
        }
        let mut result = self.gives.create();
        if let Some(f) = &self.given_item_modifier {
            result = ev.apply_fn(f, result);
            if result.is_empty() {
                return None;
            }
        }
        let mut extra = 0;
        if let Some(&n) = result.get(keys::ADDITIONAL_TRADE_COST) {
            extra += n;
            result.remove(kiln_item::component::ids::ADDITIONAL_TRADE_COST);
        }
        if let Some(set) = &self.double_trade_price_enchantments
            && result.get(keys::STORED_ENCHANTMENTS).is_some_and(|e| e.0.iter().any(|(id, _)| set.contains(*id)))
        {
            extra *= 2;
        }
        let cost_a = self.wants.to_item_cost(ev, extra);
        if cost_a.count < 1 {
            return None;
        }
        let cost_b = self.additional_wants.as_ref().map(|w| w.to_item_cost(ev, 0));
        if cost_b.as_ref().is_some_and(|b| b.count < 1) {
            return None;
        }
        let max_uses = ev.int(&self.max_uses).max(1);
        let xp = ev.int(&self.xp).max(0);
        let discount = ev.float(&self.reputation_discount).max(0.0);
        Some(MerchantOffer::new(cost_a, cost_b, result, max_uses, xp, discount))
    }
}

/// `TradeSet`.
#[derive(Debug, Clone)]
pub struct TradeSet {
    /// Trade ids in holder set order (tag order).
    pub trades: Vec<Identifier>,
    pub amount: Ref<IntProvider>,
    pub allow_duplicates: bool,
    pub random_sequence: Option<Identifier>,
}

impl TradeSet {
    fn parse(p: &Parser, j: &Json) -> PResult<TradeSet> {
        let trades = req(j, "trades", |v| match v {
            Json::Str(s) if s.starts_with('#') => {
                let tag = Identifier::parse(&s[1..]).ok_or_else(|| ParseError::new(format!("invalid tag {s:?}")))?;
                p.tags.get("minecraft:villager_trade", &tag).map(<[Identifier]>::to_vec).ok_or_else(|| ParseError::new(format!("missing tag #{tag}")))
            }
            Json::Arr(items) => items.iter().map(ident).collect(),
            _ => Ok(vec![ident(v)?]),
        })?;
        Ok(TradeSet {
            trades,
            amount: req(j, "amount", |v| IntProvider::parse_ref(p, v))?,
            allow_duplicates: opt_or(j, "allow_duplicates", false, crate::parse::boolean)?,
            random_sequence: opt(j, "random_sequence", ident)?,
        })
    }
}

/// Every trade set and villager trade of a datapack.
#[derive(Debug, Default)]
pub struct Trades {
    pub sets: HashMap<Identifier, TradeSet>,
    pub trades: HashMap<Identifier, VillagerTrade>,
    /// Files that failed to decode (`<dir>/<id>: error`).
    pub errors: Vec<String>,
}

impl Trades {
    /// Loads `trade_set/` and `villager_trade/` of `packs` against `data` (its tags, modifiers,
    /// predicates and providers). Files that fail are listed in `errors`.
    pub fn load(packs: &[&Path], data: &LootData) -> Trades {
        let mut out = Trades::default();
        let parser = Parser { names: &data.names, tags: &data.tags };
        for (dir, is_set) in [("villager_trade", false), ("trade_set", true)] {
            let files = match crate::data::list_pack_files(packs, dir) {
                Ok(f) => f,
                Err(e) => {
                    out.errors.push(format!("{dir}: {e}"));
                    continue;
                }
            };
            for (id, path) in files {
                let parsed = std::fs::read_to_string(&path)
                    .map_err(|e| ParseError::new(e.to_string()))
                    .and_then(|t| Json::parse(&t).map_err(|e| ParseError::new(e.to_string())));
                let r = parsed.and_then(|j| {
                    if is_set {
                        TradeSet::parse(&parser, &j).map(|s| {
                            out.sets.insert(id.clone(), s);
                        })
                    } else {
                        VillagerTrade::parse(&parser, &j).map(|t| {
                            out.trades.insert(id.clone(), t);
                        })
                    }
                });
                if let Err(e) = r {
                    out.errors.push(format!("{dir}/{id}: {e}"));
                }
            }
        }
        out
    }

    /// `addOffersFromTradeSet`: the offers trade set `set` rolls with `rng` (vanilla: the set's
    /// random sequence, else the level's random) for the merchant in `ctx`.
    pub fn offers(&self, data: &LootData, set: &Identifier, ctx: &dyn LootContext, rng: &mut dyn RandomSource) -> Vec<MerchantOffer> {
        let Some(set) = self.sets.get(set) else { return Vec::new() };
        let mut ev = Eval::new(data, ctx, rng);
        let count = ev.int(&set.amount);
        let mut pool: Vec<&Identifier> = set.trades.iter().collect();
        let mut out = Vec::new();
        let mut added = 0;
        while added < count && !pool.is_empty() {
            let i = ev.rng.next_int_bounded(pool.len() as i32) as usize;
            if set.allow_duplicates {
                match self.trades.get(pool[i]).and_then(|t| t.offer(&mut ev)) {
                    Some(o) => {
                        out.push(o);
                        added += 1;
                    }
                    None => {
                        pool.remove(i);
                    }
                }
            } else {
                let id = pool.remove(i);
                if let Some(o) = self.trades.get(id).and_then(|t| t.offer(&mut ev)) {
                    out.push(o);
                    added += 1;
                }
            }
        }
        out
    }
}

/// The loot context of a trade roll (`LootContextParamSets.VILLAGER_TRADE`): the merchant as
/// `this` at `origin`, with additional costs allowed. Entity predicates see the villager's
/// type (`minecraft:villager/variant`) and entity type.
pub struct TradeContext {
    pub origin: [f64; 3],
    pub entity_type: &'static str,
    /// `minecraft:villager_type` entry.
    pub villager_type: &'static str,
}

impl LootContext for TradeContext {
    fn has_entity(&self, target: EntityTarget) -> bool {
        target == EntityTarget::This
    }
    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.origin)
    }
    fn additional_cost_component_allowed(&self) -> bool {
        true
    }
    fn entity_matches(&self, target: EntityTarget, predicate: &crate::predicate::EntityPredicate) -> bool {
        if target != EntityTarget::This {
            return false;
        }
        let Some(fields) = predicate.json.as_object() else { return false };
        fields.iter().all(|(k, v)| match k.strip_prefix("minecraft:").unwrap_or(k) {
            "predicates" => v.as_object().is_some_and(|preds| {
                preds.iter().all(|(pk, pv)| match pk.strip_prefix("minecraft:").unwrap_or(pk) {
                    "villager/variant" => match pv {
                        Json::Str(s) if s.starts_with('#') => false,
                        Json::Str(s) => s == self.villager_type,
                        Json::Arr(list) => list.iter().any(|x| x.as_str() == Some(self.villager_type)),
                        _ => false,
                    },
                    _ => false,
                })
            }),
            "entity_type" => v.as_str() == Some(self.entity_type),
            _ => false,
        })
    }
}

/// A `minecraft:trade_set` id from its string form.
pub fn set_id(set: &str) -> PResult<Identifier> {
    Identifier::parse(set).map_or_else(|| fail(format!("invalid trade set {set:?}")), Ok)
}
