//! `ItemPredicate` and `DataComponentMatchers` evaluation over kiln-item's predicate model.

use crate::tags::Tags;
use kiln_item::component::{
    CollectionPredicate, DataComponentMatchers, EnchantmentPredicate, Enchantments, IntBounds, ItemPredicate,
    PartialPredicate, ids, keys,
};
use kiln_item::registry::{self, Registry};
use kiln_item::{Component, ComponentId, HolderSet, ItemStack, ItemStackTemplate};
use kiln_proto::nbt::Tag;

/// Something with data components (`DataComponentGetter`).
pub trait Components {
    fn component(&self, id: ComponentId) -> Option<&Component>;
}

impl Components for ItemStack {
    fn component(&self, id: ComponentId) -> Option<&Component> {
        ItemStack::component(self, id)
    }
}

/// A plain component map (a block entity's `collectComponents()`).
impl Components for [Component] {
    fn component(&self, id: ComponentId) -> Option<&Component> {
        self.iter().find(|c| c.id() == id)
    }
}

/// `MinMaxBounds.Ints.matches`.
pub fn int_bounds(b: &IntBounds, v: i32) -> bool {
    b.min.is_none_or(|m| m <= v) && b.max.is_none_or(|m| v <= m)
}

/// `HolderSet.contains` with tags resolved from the datapack.
pub fn holder_set_contains(tags: &Tags, reg: Registry, set: &HolderSet, id: i32) -> bool {
    match set {
        HolderSet::Direct(ids) => ids.contains(&id),
        HolderSet::Tag(tag) => tags.ids(reg, tag).is_some_and(|s| s.contains(id)),
    }
}

/// `ItemPredicate.test`.
pub fn item_matches(tags: &Tags, p: &ItemPredicate, stack: &ItemStack) -> bool {
    if let Some(items) = &p.items
        && !holder_set_contains(tags, registry::ITEM, items, stack.item())
    {
        return false;
    }
    int_bounds(&p.count, stack.count()) && components_match(tags, &p.components, stack)
}

fn template_matches(tags: &Tags, p: &ItemPredicate, t: &ItemStackTemplate) -> bool {
    item_matches(tags, p, &t.create())
}

/// `DataComponentMatchers.test`: every exact component equal, every partial predicate true.
pub fn components_match<C: Components + ?Sized>(tags: &Tags, m: &DataComponentMatchers, target: &C) -> bool {
    m.exact.iter().all(|c| target.component(c.id()) == Some(c)) && m.partial.iter().all(|p| partial_matches(tags, p, target))
}

/// `CollectionPredicate.test`.
fn collection<T, P>(c: &CollectionPredicate<P>, items: &[T], test: impl Fn(&P, &T) -> bool) -> bool {
    if let Some(contains) = &c.contains {
        // `CollectionContentsPredicate`: each test must match some element; an element may
        // satisfy several tests.
        let mut remaining: Vec<&P> = contains.iter().collect();
        if !remaining.is_empty() {
            let mut done = false;
            for item in items {
                remaining.retain(|p| !test(p, item));
                if remaining.is_empty() {
                    done = true;
                    break;
                }
            }
            if !done {
                return false;
            }
        }
    }
    if let Some(counts) = &c.count
        && !counts.iter().all(|(p, bounds)| int_bounds(bounds, items.iter().filter(|i| test(p, i)).count() as i32))
    {
        return false;
    }
    if let Some(size) = &c.size
        && !int_bounds(size, items.len() as i32)
    {
        return false;
    }
    true
}

/// `EnchantmentPredicate.containedIn`.
fn enchantment_contained(tags: &Tags, p: &EnchantmentPredicate, e: &Enchantments) -> bool {
    let any = p.levels.min.is_none() && p.levels.max.is_none();
    match &p.enchantments {
        Some(set) => {
            let ids: Vec<i32> = match set {
                HolderSet::Direct(ids) => ids.clone(),
                HolderSet::Tag(tag) => tags.ids(registry::ENCHANTMENT, tag).map(|s| s.ids().to_vec()).unwrap_or_default(),
            };
            ids.into_iter().any(|id| {
                let level = e.level(id);
                level != 0 && (any || int_bounds(&p.levels, level))
            })
        }
        None if !any => e.0.iter().any(|(_, l)| int_bounds(&p.levels, *l)),
        None => !e.is_empty(),
    }
}

fn nbt_compound(tag: &Tag) -> bool {
    matches!(tag, Tag::Compound(_))
}

/// `DataComponentPredicate.matches` for each partial predicate type.
pub fn partial_matches<C: Components + ?Sized>(tags: &Tags, p: &PartialPredicate, target: &C) -> bool {
    let get = |id: ComponentId| target.component(id);
    match p {
        PartialPredicate::Exists(id) => get(*id).is_some(),
        PartialPredicate::Damage(d) => {
            let Some(Component::Damage(damage)) = get(ids::DAMAGE) else { return false };
            let max = match get(ids::MAX_DAMAGE) {
                Some(Component::MaxDamage(m)) => *m,
                _ => 0,
            };
            int_bounds(&d.durability, max - damage) && int_bounds(&d.damage, *damage)
        }
        PartialPredicate::Enchantments(list) => match get(ids::ENCHANTMENTS) {
            Some(Component::Enchantments(e)) => list.iter().all(|p| enchantment_contained(tags, p, e)),
            _ => false,
        },
        PartialPredicate::StoredEnchantments(list) => match get(ids::STORED_ENCHANTMENTS) {
            Some(Component::StoredEnchantments(e)) => list.iter().all(|p| enchantment_contained(tags, p, e)),
            _ => false,
        },
        PartialPredicate::PotionContents(pp) => {
            let Some(Component::PotionContents(pc)) = get(ids::POTION_CONTENTS) else { return false };
            if let Some(set) = &pp.potions
                && !pc.potion.is_some_and(|id| holder_set_contains(tags, registry::POTION, set, id))
            {
                return false;
            }
            // Effect predicates need the potion's own effects (the potion registry), which kiln
            // does not model: only the custom effects are checked.
            match &pp.effects {
                None => true,
                Some(c) => collection(c, &pc.custom_effects, |mp, effect| {
                    mp.0.iter().all(|(id, ip)| {
                        effect.effect == *id
                            && int_bounds(&ip.amplifier, effect.details.amplifier)
                            && int_bounds(&ip.duration, effect.details.duration)
                            && ip.ambient.is_none_or(|a| a == effect.details.ambient)
                            && ip.visible.is_none_or(|v| v == effect.details.show_particles)
                    })
                }),
            }
        }
        PartialPredicate::CustomData(nbt) => {
            let empty = Tag::Compound(Vec::new());
            let actual = match get(ids::CUSTOM_DATA) {
                Some(Component::CustomData(c)) => &c.0,
                _ => &empty,
            };
            nbt_compound(&nbt.0) && kiln_command::blocks::compare_nbt(&nbt.0, actual, true)
        }
        PartialPredicate::Container(c) => match get(ids::CONTAINER) {
            Some(Component::Container(contents)) => c.as_ref().is_none_or(|c| {
                let items: Vec<ItemStackTemplate> = contents.0.iter().flatten().cloned().collect();
                collection(c, &items, |p, t| template_matches(tags, p, t))
            }),
            _ => false,
        },
        PartialPredicate::BundleContents(c) => match get(ids::BUNDLE_CONTENTS) {
            Some(Component::BundleContents(b)) => {
                c.as_ref().is_none_or(|c| collection(c, &b.0, |p, t| template_matches(tags, p, t)))
            }
            _ => false,
        },
        PartialPredicate::FireworkExplosion(fp) => match get(ids::FIREWORK_EXPLOSION) {
            Some(Component::FireworkExplosion(e)) => firework_matches(fp, e),
            _ => false,
        },
        PartialPredicate::Fireworks(fp) => match get(ids::FIREWORKS) {
            Some(Component::Fireworks(f)) => {
                fp.explosions.as_ref().is_none_or(|c| collection(c, &f.explosions, firework_matches))
                    && int_bounds(&fp.flight_duration, f.flight_duration)
            }
            _ => false,
        },
        PartialPredicate::WritableBookContent(c) => match get(ids::WRITABLE_BOOK_CONTENT) {
            Some(Component::WritableBookContent(b)) => c.as_ref().is_none_or(|c| {
                let pages: Vec<&String> = b.pages.iter().map(|p| &p.raw).collect();
                collection(c, &pages, |want, page| want == *page)
            }),
            _ => false,
        },
        PartialPredicate::WrittenBookContent(wp) => match get(ids::WRITTEN_BOOK_CONTENT) {
            Some(Component::WrittenBookContent(b)) => {
                wp.author.as_ref().is_none_or(|a| *a == b.author)
                    && wp.title.as_ref().is_none_or(|t| *t == b.title.raw)
                    && int_bounds(&wp.generation, b.generation)
                    && wp.resolved.is_none_or(|r| r == b.resolved)
                    && wp.pages.as_ref().is_none_or(|c| {
                        let pages: Vec<&kiln_item::Text> = b.pages.iter().map(|p| &p.raw).collect();
                        collection(c, &pages, |want, page| want == *page)
                    })
            }
            _ => false,
        },
        PartialPredicate::AttributeModifiers(c) => match get(ids::ATTRIBUTE_MODIFIERS) {
            Some(Component::AttributeModifiers(m)) => c.as_ref().is_none_or(|c| {
                collection(c, &m.0, |p, e| {
                    p.attribute.as_ref().is_none_or(|s| holder_set_contains(tags, registry::ATTRIBUTE, s, e.attribute))
                        && p.id.as_ref().is_none_or(|id| *id == e.id)
                        && p.amount.min.is_none_or(|v| v <= e.amount)
                        && p.amount.max.is_none_or(|v| e.amount <= v)
                        && p.operation.is_none_or(|o| o == e.operation)
                        && p.slot.is_none_or(|s| s == e.slot)
                })
            }),
            _ => false,
        },
        PartialPredicate::Trim(tp) => match get(ids::TRIM) {
            Some(Component::Trim(t)) => {
                holder_in(tags, &tp.material, registry::TRIM_MATERIAL, &t.material)
                    && holder_in(tags, &tp.pattern, registry::TRIM_PATTERN, &t.pattern)
            }
            _ => false,
        },
        PartialPredicate::JukeboxPlayable(set) => match get(ids::JUKEBOX_PLAYABLE) {
            Some(Component::JukeboxPlayable(j)) => match (set, &j.0) {
                (None, _) => true,
                (Some(set), kiln_item::Holder::Reference(id)) => holder_set_contains(tags, registry::JUKEBOX_SONG, set, *id),
                (Some(_), kiln_item::Holder::Direct(_)) => false,
            },
            _ => false,
        },
        PartialPredicate::VillagerVariant(set) => match target.component(ids::VILLAGER_VARIANT) {
            Some(Component::VillagerVariant(v)) => holder_set_contains(tags, registry::VILLAGER_TYPE, set, v.0),
            _ => false,
        },
    }
}

/// An optional holder set test against a holder (inline values never match a set).
fn holder_in<T>(tags: &Tags, set: &Option<HolderSet>, reg: Registry, h: &kiln_item::Holder<T>) -> bool {
    match (set, h) {
        (None, _) => true,
        (Some(set), kiln_item::Holder::Reference(id)) => holder_set_contains(tags, reg, set, *id),
        (Some(_), kiln_item::Holder::Direct(_)) => false,
    }
}

fn firework_matches(p: &kiln_item::component::FireworkPredicate, e: &kiln_item::component::FireworkExplosion) -> bool {
    p.shape.is_none_or(|s| s == e.shape)
        && p.has_twinkle.is_none_or(|t| t == e.has_twinkle)
        && p.has_trail.is_none_or(|t| t == e.has_trail)
}

/// The `enchantments` component of a stack, if it has one.
pub fn enchantments(stack: &ItemStack) -> Option<&Enchantments> {
    stack.get(keys::ENCHANTMENTS)
}
