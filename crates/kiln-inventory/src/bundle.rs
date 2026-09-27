//! Bundles' click behavior (`BundleItem.overrideStackedOnOther` / `overrideOtherStackedOnMe`,
//! `BundleContents.Mutable`), with vanilla's exact fractional weights.
//!
//! The selected-item index (`bundle_item_selected`) is not part of the component on the wire or
//! in saves; kiln-item does not keep it, so it is always "none" here and removal takes the
//! first stack.

use crate::click::ClickAction;
use crate::menu::{ClickCrash, Env, Menu};
use crate::slot::can_fit_inside_container_items;
use crate::stack::{StackExt, create_checked, same_item_same_components};
use kiln_item::component::BundleContents;
use kiln_item::{ItemStack, ItemStackTemplate, keys};

/// `org.apache.commons.lang3.math.Fraction` in lowest terms; `None` stands for the
/// `ArithmeticException` of an int overflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Frac {
    num: i64,
    den: i64,
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a.abs() } else { gcd(b, a % b) }
}

impl Frac {
    const ZERO: Frac = Frac { num: 0, den: 1 };
    const ONE: Frac = Frac { num: 1, den: 1 };

    fn new(num: i64, den: i64) -> Option<Frac> {
        if den == 0 {
            return None;
        }
        let g = gcd(num, den).max(1);
        let (mut num, mut den) = (num / g, den / g);
        if den < 0 {
            num = -num;
            den = -den;
        }
        (i32::try_from(num).is_ok() && i32::try_from(den).is_ok()).then_some(Frac { num, den })
    }

    fn add(self, o: Frac) -> Option<Frac> {
        Frac::new(self.num * o.den + o.num * self.den, self.den * o.den)
    }

    fn sub(self, o: Frac) -> Option<Frac> {
        Frac::new(self.num * o.den - o.num * self.den, self.den * o.den)
    }

    fn mul(self, n: i64) -> Option<Frac> {
        Frac::new(self.num * n, self.den)
    }

    fn div(self, o: Frac) -> Option<Frac> {
        Frac::new(self.num * o.den, self.den * o.num)
    }

    /// `intValue`: truncated toward zero.
    fn int(self) -> i64 {
        self.num / self.den
    }

}

/// `BundleContents.getWeight(item)`: a nested bundle weighs its contents plus 1/16, a beehive with
/// bees a whole bundle, anything else one over its maximum stack size.
fn weight(stack: &ItemStack) -> Option<Frac> {
    if let Some(contents) = stack.get(keys::BUNDLE_CONTENTS) {
        return content_weight(contents)?.add(Frac::new(1, 16)?);
    }
    if stack.get(keys::BEES).is_some_and(|b| !b.0.is_empty()) {
        return Some(Frac::ONE);
    }
    Frac::new(1, stack.max_stack_size() as i64)
}

/// `BundleContents.computeContentWeight`.
fn content_weight(contents: &BundleContents) -> Option<Frac> {
    contents.0.iter().try_fold(Frac::ZERO, |acc, t| {
        let s = t.create();
        acc.add(weight(&s)?.mul(t.count as i64)?)
    })
}

/// `BundleContents.canItemBeInBundle`.
fn can_be_in_bundle(stack: &ItemStack) -> bool {
    !stack.is_empty() && can_fit_inside_container_items(stack)
}

/// `BundleContents.Mutable`.
struct Mutable {
    items: Vec<ItemStack>,
    weight: Frac,
}

impl Mutable {
    /// `BundleContents.asMutable` (an unweighable bundle opens as an empty one).
    fn of(contents: &BundleContents) -> Mutable {
        match content_weight(contents) {
            Some(weight) => Mutable { items: contents.0.iter().map(create_checked).collect(), weight },
            None => Mutable { items: Vec::new(), weight: Frac::ZERO },
        }
    }

    fn max_to_add(&self, w: Frac) -> i32 {
        Frac::ONE.sub(self.weight).and_then(|free| free.div(w)).map_or(0, |f| f.int().max(0) as i32)
    }

    fn find(&self, stack: &ItemStack) -> Option<usize> {
        if !stack.is_stackable() {
            return None;
        }
        self.items.iter().position(|s| same_item_same_components(s, stack))
    }

    /// `tryInsert`: moves what fits of `stack` in; returns how many.
    fn try_insert(&mut self, stack: &mut ItemStack) -> i32 {
        if !can_be_in_bundle(stack) {
            return 0;
        }
        let Some(w) = weight(stack) else { return 0 };
        let n = stack.count().min(self.max_to_add(w));
        if n == 0 {
            return 0;
        }
        let Some(new_weight) = w.mul(n as i64).and_then(|a| self.weight.add(a)) else { return 0 };
        self.weight = new_weight;
        match self.find(stack) {
            Some(i) => {
                let old = self.items.remove(i);
                let merged = old.copy_with_count(old.count() + n);
                stack.shrink_count(n);
                self.items.insert(0, merged);
            }
            None => {
                let part = stack.split_count(n);
                self.items.insert(0, part);
            }
        }
        n
    }

    /// `removeOne`: the selected stack (none selected: the first).
    fn remove_one(&mut self) -> Option<ItemStack> {
        if self.items.is_empty() {
            return None;
        }
        let removed = self.items.remove(0).copy();
        if let Some(w) = weight(&removed).and_then(|w| w.mul(removed.count() as i64)).and_then(|w| self.weight.sub(w)) {
            self.weight = w;
        }
        Some(removed)
    }

    /// `toImmutable`; vanilla throws on an empty stack left in the list (from a template that
    /// did not validate).
    fn to_component(&self) -> Result<BundleContents, ClickCrash> {
        if self.items.iter().any(ItemStack::is_empty) {
            return Err(ClickCrash { slot: -1 });
        }
        Ok(BundleContents(self.items.iter().map(ItemStackTemplate::from_stack).collect()))
    }
}

/// `AbstractContainerMenu.tryItemClickBehaviourOverride`: whether the carried stack or the
/// slot's stack (a bundle) handled the click itself.
pub(crate) fn click_override(menu: &mut Menu, env: &mut Env, slot: usize, action: ClickAction) -> Result<bool, ClickCrash> {
    if stacked_on_other(menu, env, slot, action)? {
        return Ok(true);
    }
    other_stacked_on_me(menu, env, slot, action)
}

/// `BundleItem.overrideStackedOnOther`: the carried bundle clicked onto a slot.
fn stacked_on_other(menu: &mut Menu, env: &mut Env, slot: usize, action: ClickAction) -> Result<bool, ClickCrash> {
    let Some(contents) = menu.carried.get(keys::BUNDLE_CONTENTS).cloned() else { return Ok(false) };
    let other_empty = menu.item(env, slot).is_empty();
    let mut m = Mutable::of(&contents);
    match action {
        ClickAction::Primary if !other_empty => {
            let other = menu.item(env, slot).clone();
            if let Some(w) = weight(&other) {
                let max = m.max_to_add(w);
                if can_be_in_bundle(&other) {
                    let mut taken = menu.safe_take(env, slot, other.count(), max);
                    m.try_insert(&mut taken);
                }
            }
        }
        ClickAction::Secondary if other_empty => {
            if let Some(removed) = m.remove_one() {
                let n = removed.count();
                let mut rest = menu.safe_insert(env, slot, removed, n);
                if rest.count() > 0 {
                    m.try_insert(&mut rest);
                }
            }
        }
        _ => return Ok(false),
    }
    menu.carried.set(keys::BUNDLE_CONTENTS.wrap(m.to_component()?));
    menu.slots_changed(env);
    Ok(true)
}

/// `BundleItem.overrideOtherStackedOnMe`: a stack (or nothing) clicked onto a bundle in a slot.
fn other_stacked_on_me(menu: &mut Menu, env: &mut Env, slot: usize, action: ClickAction) -> Result<bool, ClickCrash> {
    let carried_empty = menu.carried.is_empty();
    let Some(contents) = menu.item(env, slot).get(keys::BUNDLE_CONTENTS).cloned() else { return Ok(false) };
    if action == ClickAction::Primary && carried_empty {
        // toggleSelectedItem(-1): rewrites the component (an unweighable bundle empties).
        let m = Mutable::of(&contents);
        menu.item_mut(env, slot).set(keys::BUNDLE_CONTENTS.wrap(m.to_component()?));
        return Ok(false);
    }
    let mut m = Mutable::of(&contents);
    match action {
        ClickAction::Primary => {
            if menu.allow_modification(env, slot) {
                let mut carried = std::mem::take(&mut menu.carried);
                m.try_insert(&mut carried);
                menu.carried = carried;
            }
        }
        ClickAction::Secondary if carried_empty => {
            if menu.allow_modification(env, slot)
                && let Some(removed) = m.remove_one()
            {
                menu.carried = removed;
            }
        }
        ClickAction::Secondary => {
            menu.item_mut(env, slot).set(keys::BUNDLE_CONTENTS.wrap(Mutable::of(&contents).to_component()?));
            return Ok(false);
        }
    }
    menu.item_mut(env, slot).set(keys::BUNDLE_CONTENTS.wrap(m.to_component()?));
    menu.slots_changed(env);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_reduce_and_compare() {
        let a = Frac::new(1, 64).unwrap().mul(32).unwrap();
        assert_eq!(a, Frac::new(1, 2).unwrap());
        assert_eq!(Frac::ONE.sub(a).unwrap().div(Frac::new(1, 16).unwrap()).unwrap().int(), 8);
    }

    #[test]
    fn bundles_fill_by_weight() {
        let mut m = Mutable::of(&BundleContents(Vec::new()));
        let mut pearls = ItemStack::of("ender_pearl", 20).unwrap();
        assert_eq!(m.try_insert(&mut pearls), 16);
        assert_eq!(pearls.count(), 4);
        let mut stone = ItemStack::of("stone", 5).unwrap();
        assert_eq!(m.try_insert(&mut stone), 0);
        assert_eq!(m.remove_one().map(|s| s.count()), Some(16));
    }
}
