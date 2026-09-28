//! Villager trading data (`net.minecraft.world.item.trading`): [`ItemCost`], [`MerchantOffer`]
//! and the offer list, with their NBT codecs (`Offers:{Recipes:[...]}` on villagers) and network
//! codecs (`ClientboundMerchantOffersPacket`).

use crate::patch::DataComponentPatch;
use crate::registry;
use crate::stack::ItemStack;
use crate::value::{DataResult, MapBuilder, Value};
use bytes::{BufMut, Bytes, BytesMut};
use kiln_proto::WriteExt;
use kiln_proto::nbt::Tag;

/// `ItemCost`: an item, a count and an exact component predicate (the added components of
/// `components`).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemCost {
    pub item: i32,
    pub count: i32,
    pub components: DataComponentPatch,
}

impl ItemCost {
    pub fn new(item: i32, count: i32) -> ItemCost {
        ItemCost { item, count, components: DataComponentPatch::new() }
    }

    /// `itemStack()`: the item with the count and the predicate's components.
    pub fn stack(&self) -> ItemStack {
        ItemStack::from_parts(self.item, self.count, self.components.clone())
    }

    /// `test`: same item and every predicate component present with that value (count is not
    /// checked).
    pub fn test(&self, stack: &ItemStack) -> bool {
        !stack.is_empty() && stack.item() == self.item && self.components.added().all(|c| stack.component(c.id()) == Some(c))
    }

    /// `ItemCost.STREAM_CODEC`: item id, VarInt count, `DataComponentExactPredicate` (a list of
    /// typed components).
    pub fn write(&self, out: &mut BytesMut) {
        out.put_varint(self.item);
        out.put_varint(self.count);
        out.put_varint(self.components.added().count() as i32);
        for c in self.components.added() {
            out.put_varint(c.id() as i32);
            c.write(out);
        }
    }

    /// `ItemCost.CODEC`: `{id, count (always written), components (when not empty)}`.
    pub fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.put("id", registry::ITEM.id_to_value(self.item)).put("count", Value::Int(self.count));
        if !self.components.is_empty() {
            m.put("components", self.components.to_value());
        }
        m.build()
    }

    pub fn from_value(v: &Value) -> DataResult<ItemCost> {
        let m = v.as_map()?;
        let item = m.req_with("id", |v| registry::ITEM.id_from_value(v))?;
        let count = m.opt_or("count", 1, Value::as_i32)?;
        let components = m.opt_or("components", DataComponentPatch::new(), DataComponentPatch::from_value)?;
        Ok(ItemCost { item, count, components })
    }
}

/// `MerchantOffer`.
#[derive(Debug, Clone, PartialEq)]
pub struct MerchantOffer {
    pub cost_a: ItemCost,
    pub cost_b: Option<ItemCost>,
    pub result: ItemStack,
    pub uses: i32,
    pub max_uses: i32,
    pub reward_exp: bool,
    pub special_price_diff: i32,
    pub demand: i32,
    pub price_multiplier: f32,
    pub xp: i32,
}

impl MerchantOffer {
    /// `new MerchantOffer(costA, costB, result, maxUses, xp, priceMultiplier)`.
    pub fn new(cost_a: ItemCost, cost_b: Option<ItemCost>, result: ItemStack, max_uses: i32, xp: i32, price_multiplier: f32) -> MerchantOffer {
        MerchantOffer { cost_a, cost_b, result, uses: 0, max_uses, reward_exp: true, special_price_diff: 0, demand: 0, price_multiplier, xp }
    }

    /// `getModifiedCostCount(baseCostA)`: demand and special prices, clamped to [1, max stack].
    pub fn modified_cost_a_count(&self) -> i32 {
        let base = self.cost_a.count;
        let demand_bonus = (((base * self.demand) as f32 * self.price_multiplier).floor() as i32).max(0);
        (base + demand_bonus + self.special_price_diff).clamp(1, self.cost_a.stack().max_stack_size())
    }

    /// `getCostA`: the first cost at its current price.
    pub fn cost_a(&self) -> ItemStack {
        self.cost_a.stack().with_count(self.modified_cost_a_count())
    }

    /// `getCostB` (never price-adjusted); empty without a second cost.
    pub fn cost_b(&self) -> ItemStack {
        self.cost_b.as_ref().map_or_else(ItemStack::empty, ItemCost::stack)
    }

    pub fn is_out_of_stock(&self) -> bool {
        self.uses >= self.max_uses
    }

    pub fn increase_uses(&mut self) {
        self.uses += 1;
    }

    pub fn reset_uses(&mut self) {
        self.uses = 0;
    }

    pub fn needs_restock(&self) -> bool {
        self.uses > 0
    }

    /// `updateDemand` (at restock).
    pub fn update_demand(&mut self) {
        self.demand = self.demand + self.uses - (self.max_uses - self.uses);
    }

    /// `satisfiedBy(a, b)`.
    pub fn satisfied_by(&self, a: &ItemStack, b: &ItemStack) -> bool {
        if !self.cost_a.test(a) || a.count() < self.modified_cost_a_count() {
            return false;
        }
        match &self.cost_b {
            Some(cb) => cb.test(b) && b.count() >= cb.count,
            None => b.is_empty(),
        }
    }

    /// `take(a, b)`: shrinks the payments by the prices when they satisfy the offer.
    pub fn take(&self, a: &mut ItemStack, b: &mut ItemStack) -> bool {
        if !self.satisfied_by(a, b) {
            return false;
        }
        a.shrink(self.cost_a().count());
        let cb = self.cost_b();
        if !cb.is_empty() {
            b.shrink(cb.count());
        }
        true
    }

    /// `MerchantOffer.STREAM_CODEC` (the reward flag is not sent).
    pub fn write(&self, out: &mut BytesMut) {
        self.cost_a.write(out);
        self.result.write(out);
        out.put_u8(self.cost_b.is_some() as u8);
        if let Some(b) = &self.cost_b {
            b.write(out);
        }
        out.put_u8(self.is_out_of_stock() as u8);
        out.put_i32(self.uses);
        out.put_i32(self.max_uses);
        out.put_i32(self.xp);
        out.put_i32(self.special_price_diff);
        out.put_f32(self.price_multiplier);
        out.put_i32(self.demand);
    }

    /// `MerchantOffer.CODEC`.
    pub fn to_value(&self) -> Value {
        let mut m = MapBuilder::new();
        m.put("buy", self.cost_a.to_value());
        if let Some(b) = &self.cost_b {
            m.put("buyB", b.to_value());
        }
        m.put("sell", self.result.to_value())
            .put("uses", Value::Int(self.uses))
            .put("maxUses", Value::Int(self.max_uses))
            .put("rewardExp", Value::Bool(self.reward_exp))
            .put("specialPrice", Value::Int(self.special_price_diff))
            .put("demand", Value::Int(self.demand))
            .put("priceMultiplier", Value::Float(self.price_multiplier))
            .put("xp", Value::Int(self.xp));
        m.build()
    }

    pub fn from_value(v: &Value) -> DataResult<MerchantOffer> {
        let m = v.as_map()?;
        Ok(MerchantOffer {
            cost_a: m.req_with("buy", ItemCost::from_value)?,
            cost_b: m.lenient_or("buyB", None, |v| ItemCost::from_value(v).map(Some)),
            result: m.req_with("sell", ItemStack::from_value)?,
            uses: m.opt_or("uses", 0, Value::as_i32)?,
            max_uses: m.opt_or("maxUses", 4, Value::as_i32)?,
            reward_exp: m.opt_or("rewardExp", true, Value::as_bool)?,
            special_price_diff: m.opt_or("specialPrice", 0, Value::as_i32)?,
            demand: m.opt_or("demand", 0, Value::as_i32)?,
            price_multiplier: m.opt_or("priceMultiplier", 0.0, Value::as_f32)?,
            xp: m.opt_or("xp", 1, Value::as_i32)?,
        })
    }
}

/// `MerchantOffers.getRecipeFor(a, b, hint)`: with `0 < hint < size` only that offer is tried;
/// otherwise the first satisfied offer. Out-of-stock offers count.
pub fn recipe_for(offers: &[MerchantOffer], a: &ItemStack, b: &ItemStack, hint: i32) -> Option<usize> {
    if hint > 0 && (hint as usize) < offers.len() {
        return offers[hint as usize].satisfied_by(a, b).then_some(hint as usize);
    }
    offers.iter().position(|o| o.satisfied_by(a, b))
}

/// `MerchantOffers.CODEC` as NBT: `{Recipes:[...]}` (offers that fail to encode are skipped).
pub fn offers_to_nbt(offers: &[MerchantOffer]) -> Tag {
    let list = Value::List(offers.iter().map(MerchantOffer::to_value).collect());
    let mut m = MapBuilder::new();
    m.put("Recipes", list);
    m.build().to_nbt()
}

/// Reads `{Recipes:[...]}`; a malformed list reads as no offers.
pub fn offers_from_nbt(tag: &Tag) -> Vec<MerchantOffer> {
    let v = Value::from_nbt(tag);
    let Ok(m) = v.as_map() else { return Vec::new() };
    let Some(list) = m.get("Recipes") else { return Vec::new() };
    let Ok(list) = list.as_list() else { return Vec::new() };
    list.iter().filter_map(|o| MerchantOffer::from_value(o).ok()).collect()
}

/// `ClientboundMerchantOffersPacket`: container id, the offers, the villager's level and xp, and
/// whether the client shows the progress bar and the restock hint.
pub fn merchant_offers_packet(container_id: i32, offers: &[MerchantOffer], level: i32, xp: i32, show_progress: bool, can_restock: bool) -> Bytes {
    let mut b = BytesMut::with_capacity(64 + offers.len() * 32);
    b.put_varint(kiln_data::packets::play::clientbound::MERCHANT_OFFERS);
    b.put_varint(container_id);
    b.put_varint(offers.len() as i32);
    for o in offers {
        o.write(&mut b);
    }
    b.put_varint(level);
    b.put_varint(xp);
    b.put_u8(show_progress as u8);
    b.put_u8(can_restock as u8);
    b.freeze()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emerald(n: i32) -> ItemCost {
        ItemCost::new(registry::ITEM.id("minecraft:emerald").unwrap(), n)
    }

    #[test]
    fn prices_and_payment() {
        let wheat = registry::ITEM.id("minecraft:wheat").unwrap();
        let mut o = MerchantOffer::new(ItemCost::new(wheat, 20), None, ItemStack::of("minecraft:emerald", 1).unwrap(), 16, 2, 0.05);
        let mut a = ItemStack::new(wheat, 25);
        let mut b = ItemStack::empty();
        assert!(o.satisfied_by(&a, &b));
        assert!(o.take(&mut a, &mut b));
        assert_eq!(a.count(), 5);
        assert!(!o.satisfied_by(&a, &b));
        // Demand raises the first price: 20 + floor(20 * 5 * 0.05) = 25.
        o.demand = 5;
        assert_eq!(o.modified_cost_a_count(), 25);
        o.special_price_diff = -30;
        assert_eq!(o.modified_cost_a_count(), 1);
    }

    #[test]
    fn nbt_round_trip() {
        let o = MerchantOffer::new(emerald(3), Some(ItemCost::new(registry::ITEM.id("minecraft:book").unwrap(), 1)), ItemStack::of("minecraft:bookshelf", 1).unwrap(), 12, 5, 0.2);
        let tag = offers_to_nbt(std::slice::from_ref(&o));
        assert_eq!(offers_from_nbt(&tag), vec![o]);
    }

    #[test]
    fn hint_only_tries_that_offer() {
        let wheat = registry::ITEM.id("minecraft:wheat").unwrap();
        let offers = vec![
            MerchantOffer::new(ItemCost::new(wheat, 1), None, ItemStack::of("minecraft:emerald", 1).unwrap(), 16, 2, 0.05),
            MerchantOffer::new(emerald(1), None, ItemStack::of("minecraft:bread", 6).unwrap(), 16, 1, 0.05),
        ];
        let a = ItemStack::new(wheat, 1);
        assert_eq!(recipe_for(&offers, &a, &ItemStack::empty(), 0), Some(0));
        assert_eq!(recipe_for(&offers, &a, &ItemStack::empty(), 1), None);
    }
}
