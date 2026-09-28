//! Where criteria fire (`CriteriaTriggers.*.trigger` at vanilla's call sites) and the serial
//! upkeep that grants what completed advancements give: rewards (experience, loot, recipes,
//! a function run as the player), the chat announcement (`show_advancement_messages`) and
//! Update Advancements.

use super::criteria::{self, Criterion, Subject, Trigger, TriggerCtx, WorldProbe};
use super::progress::{PlayerAdvancements, now_millis};
use crate::{Player, Sim};
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;
use kiln_link::ConnId;
use kiln_loot::LootData;
use std::sync::{Arc, OnceLock};

fn empty_loot() -> &'static LootData {
    static EMPTY: OnceLock<LootData> = OnceLock::new();
    EMPTY.get_or_init(LootData::default)
}

/// Blocks, biomes, light and structures of a region's loaded chunks.
pub(crate) struct CellProbe<'a> {
    pub cells: &'a kiln_region::CellSet<kiln_world::Cell>,
    pub min_y: i32,
    pub height: i32,
    /// `Level.getSkyDarken`.
    pub sky_darken: i32,
}

impl<'a> CellProbe<'a> {
    pub fn new(cells: &'a kiln_region::CellSet<kiln_world::Cell>, env: &crate::blocks::BlockEnv) -> Self {
        CellProbe { cells, min_y: env.min_y, height: env.height, sky_darken: env.mobs.sky_darken }
    }
}

impl WorldProbe for CellProbe<'_> {
    fn block(&self, pos: [i32; 3]) -> Option<u16> {
        use kiln_world::Blocks;
        self.cells.get_block(pos[0], pos[1], pos[2])
    }

    fn biome(&self, pos: [i32; 3]) -> Option<&'static str> {
        use kiln_world::Blocks;
        let chunk = self.cells.chunk(kiln_world::ChunkPos::of_block(pos[0], pos[2]))?;
        let rel = pos[1] - self.min_y;
        let section = chunk.sections.get(usize::try_from(rel >> 4).ok()?)?;
        let id = match &section.biomes {
            kiln_world::section::Biomes::Single(b) => b.to_owned(),
            kiln_world::section::Biomes::Cells(cells) => {
                cells[((((rel & 15) >> 2) << 4) | (((pos[2] & 15) >> 2) << 2) | ((pos[0] & 15) >> 2)) as usize]
            }
        };
        let biomes = kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == "minecraft:worldgen/biome")?.1;
        biomes.get(id as usize).copied()
    }

    fn light(&self, pos: [i32; 3]) -> Option<i32> {
        use kiln_world::Blocks;
        use kiln_world::chunk::LightLayer;
        self.cells.chunk(kiln_world::ChunkPos::of_block(pos[0], pos[2]))?;
        let top = self.min_y + self.height;
        let sky = self.cells.light_at(LightLayer::Sky, pos[0], pos[1], pos[2]).map_or(if pos[1] >= top { 15 } else { 0 }, i32::from);
        let block = self.cells.light_at(LightLayer::Block, pos[0], pos[1], pos[2]).map_or(0, i32::from);
        Some(block.max(sky - self.sky_darken))
    }

    fn can_see_sky(&self, pos: [i32; 3]) -> Option<bool> {
        use kiln_world::Blocks;
        use kiln_world::chunk::LightLayer;
        self.cells.chunk(kiln_world::ChunkPos::of_block(pos[0], pos[2]))?;
        let top = self.min_y + self.height;
        Some(self.cells.light_at(LightLayer::Sky, pos[0], pos[1], pos[2]).map_or(pos[1] >= top, |s| s >= 15))
    }

    fn structures_at(&self, pos: [i32; 3]) -> Option<Vec<String>> {
        use kiln_proto::nbt::Tag;
        use kiln_world::Blocks;
        let here = self.cells.chunk(kiln_world::ChunkPos::of_block(pos[0], pos[2]))?;
        let mut out = Vec::new();
        let Some(data) = here.structures.as_deref() else { return Some(out) };
        let Some(Tag::Compound(refs)) = data.get("References") else { return Some(out) };
        let inside = |bb: &[i32]| bb.len() == 6 && (bb[0]..=bb[3]).contains(&pos[0]) && (bb[1]..=bb[4]).contains(&pos[1]) && (bb[2]..=bb[5]).contains(&pos[2]);
        for (id, list) in refs {
            let Tag::LongArray(chunks) = list else { continue };
            let found = chunks.iter().any(|&packed| {
                // `ChunkPos.toLong`: x in the low 32 bits, z in the high.
                let c = kiln_world::ChunkPos::new(packed as i32, (packed >> 32) as i32);
                let Some(start) = self.cells.chunk(c).and_then(|ch| ch.structures.as_deref()).and_then(|s| s.get("starts")).and_then(|s| s.get(id)) else {
                    return false;
                };
                let Some(Tag::List(children)) = start.get("Children") else { return false };
                children.iter().any(|piece| matches!(piece.get("BB"), Some(Tag::IntArray(bb)) if inside(bb)))
            });
            if found {
                out.push(id.clone());
            }
        }
        Some(out)
    }
}
// Equipment slot names as entity predicates spell them.
const SLOT_NAMES: [(EquipmentSlot, &str); 6] = [
    (EquipmentSlot::MainHand, "mainhand"),
    (EquipmentSlot::OffHand, "offhand"),
    (EquipmentSlot::Head, "head"),
    (EquipmentSlot::Chest, "chest"),
    (EquipmentSlot::Legs, "legs"),
    (EquipmentSlot::Feet, "feet"),
];

impl Player {
    /// The player as criteria conditions see it.
    pub(crate) fn subject<'a>(&'a self, world: Option<&'a dyn WorldProbe>) -> Subject<'a> {
        Subject {
            type_id: kiln_item::registry::ENTITY_TYPE.id("minecraft:player").unwrap_or(-1),
            pos: self.pos,
            dim: crate::DIMENSIONS[self.dim].0,
            on_ground: self.on_ground,
            on_fire: self.fire_ticks > 0,
            sneaking: self.sneaking,
            sprinting: self.sprinting,
            flying: self.flying,
            baby: false,
            equipment: SLOT_NAMES.iter().map(|(s, n)| (*n, self.inv.equipped(*s))).collect(),
            world,
            components: Default::default(),
            effects: self.effects.values().map(|e| (e.id, e.amplifier, e.duration, e.ambient, e.visible)).collect(),
            vehicle: self.vehicle_type,
        }
    }

    /// `SimpleCriterionTrigger.trigger`: every listening criterion of `trigger` whose own
    /// conditions (`test`) and player condition hold is awarded.
    pub(crate) fn fire(
        &mut self,
        trigger: &str,
        world: Option<&dyn WorldProbe>,
        test: impl Fn(&Criterion, &LootData, &Subject) -> bool,
    ) {
        let data = self.advancements.data.clone();
        let candidates = data.criteria_for(trigger);
        // Most triggers fire every tick or on every move; nothing to test once the player has
        // every criterion that listens to them.
        if !candidates.iter().any(|&(i, c)| self.advancements.listening(i, c)) {
            return;
        }
        let loot = self.loot.clone();
        let loot: &LootData = loot.as_deref().unwrap_or_else(|| empty_loot());
        let mut hits = Vec::new();
        {
            let subject = self.subject(world);
            let ctx = TriggerCtx { this: &subject, tags: &loot.tags, origin: None, block: None, tool: None };
            for &(i, c) in candidates {
                if !self.advancements.listening(i, c) {
                    continue;
                }
                let crit = &data.list[i].criteria[c].1;
                if !test(crit, loot, &subject) {
                    continue;
                }
                if crit.player.as_ref().is_some_and(|cap| !criteria::test_cap(loot, cap, &ctx)) {
                    continue;
                }
                hits.push((i, c));
            }
        }
        if hits.is_empty() {
            return;
        }
        let now = now_millis();
        for (i, c) in hits {
            self.advancements.award(i, c, now);
        }
    }

    /// `InventoryChangeTrigger.trigger` for a changed inventory slot.
    pub(crate) fn inventory_changed(&mut self, changed: &ItemStack) {
        use kiln_inventory::Container;
        let size = self.inv.size();
        let items: Vec<ItemStack> = (0..size).map(|i| self.inv.item(i).clone()).collect();
        let (mut full, mut empty, mut occupied) = (0, 0, 0);
        for s in &items {
            if s.is_empty() {
                empty += 1;
            } else {
                occupied += 1;
                if s.count() >= s.max_stack_size() {
                    full += 1;
                }
            }
        }
        let counts = (full, empty, occupied);
        self.fire("minecraft:inventory_changed", None, |c, loot, _| criteria::inventory_matches(&loot.tags, &c.trigger, &items, changed, counts));
    }

    /// `RecipeUnlockedTrigger.trigger`.
    pub(crate) fn recipe_unlocked(&mut self, recipe: &str) {
        self.fire("minecraft:recipe_unlocked", None, |c, _, _| matches!(&c.trigger, Trigger::RecipeUnlocked { recipes } if recipes.iter().any(|r| r == recipe)));
    }

    /// `RecipeCraftedTrigger.trigger`: the recipe and the stacks it took.
    pub(crate) fn recipe_crafted(&mut self, recipe: &str, ingredients: &[ItemStack]) {
        self.fire("minecraft:recipe_crafted", None, |c, loot, _| match &c.trigger {
            Trigger::RecipeCrafted { recipes, ingredients: wanted } => {
                if !recipes.iter().any(|r| r == recipe) {
                    return false;
                }
                let mut left: Vec<&ItemStack> = ingredients.iter().filter(|s| !s.is_empty()).collect();
                wanted.iter().all(|p| match left.iter().position(|s| kiln_loot::predicate::item_matches(&loot.tags, p, s)) {
                    Some(k) => {
                        left.remove(k);
                        true
                    }
                    None => false,
                })
            }
            _ => false,
        });
    }

    /// The per-tick triggers of `ServerPlayer.tick` / `doTick`: `tick`, and `location` every
    /// 20 ticks.
    pub(crate) fn tick_triggers(&mut self, world: &dyn WorldProbe) {
        self.fire("minecraft:tick", None, |c, _, _| matches!(c.trigger, Trigger::Player));
        if self.tick_count % 20 == 0 && !self.dead {
            self.fire("minecraft:location", Some(world), |c, _, _| matches!(c.trigger, Trigger::Player));
        }
        // `LevitationTrigger`: how far and how long since the levitation began.
        if let Some((start, from)) = self.levitation_start {
            let (duration, pos) = (self.tick_count - start, self.pos);
            self.fire_conds("minecraft:levitation", None, |c, _, _| {
                c.distance("distance", from, pos) && kiln_loot::predicate::item::int_bounds(&c.ints("duration"), duration)
            });
        }
    }

    /// `ConsumeItemTrigger.trigger`.
    pub(crate) fn consumed(&mut self, stack: &ItemStack) {
        self.fire("minecraft:consume_item", None, |c, loot, _| match &c.trigger {
            Trigger::ConsumeItem { item } => item.as_ref().is_none_or(|p| kiln_loot::predicate::item_matches(&loot.tags, p, stack)),
            _ => false,
        });
    }

    /// `ItemUsedOnLocationTrigger.trigger` (`placed_block`, `item_used_on_block`): conditions
    /// on the block at `pos` (its state now) and the tool used.
    pub(crate) fn used_on_block(&mut self, trigger: &str, pos: [i32; 3], state: u16, tool: &ItemStack, world: &dyn WorldProbe) {
        let data = self.advancements.data.clone();
        if data.criteria_for(trigger).is_empty() {
            return;
        }
        let loot = self.loot.clone();
        let loot: &LootData = loot.as_deref().unwrap_or_else(|| empty_loot());
        let origin = [pos[0] as f64 + 0.5, pos[1] as f64 + 0.5, pos[2] as f64 + 0.5];
        let mut hits = Vec::new();
        {
            let subject = self.subject(Some(world));
            let player_ctx = TriggerCtx { this: &subject, tags: &loot.tags, origin: None, block: None, tool: None };
            let block_ctx = TriggerCtx { this: &subject, tags: &loot.tags, origin: Some(origin), block: Some(state), tool: Some(tool) };
            for &(i, c) in data.criteria_for(trigger) {
                if !self.advancements.listening(i, c) {
                    continue;
                }
                let crit = &data.list[i].criteria[c].1;
                let Trigger::Location { location } = &crit.trigger else { continue };
                if location.as_ref().is_some_and(|cap| !criteria::test_cap(loot, cap, &block_ctx)) {
                    continue;
                }
                if crit.player.as_ref().is_some_and(|cap| !criteria::test_cap(loot, cap, &player_ctx)) {
                    continue;
                }
                hits.push((i, c));
            }
        }
        let now = now_millis();
        for (i, c) in hits {
            self.advancements.award(i, c, now);
        }
    }

    /// `EnterBlockTrigger.trigger` for a block the player is inside.
    pub(crate) fn entered_block(&mut self, state: u16) {
        let data = self.advancements.data.clone();
        if !data.criteria_for("minecraft:enter_block").iter().any(|&(i, c)| self.advancements.listening(i, c)) {
            return;
        }
        let block = kiln_item::registry::BLOCK.id(kiln_data::builtin_entries("minecraft:block").and_then(|b| b.get(kiln_data::block_logic::block_index(state)).copied()).unwrap_or(""));
        self.fire("minecraft:enter_block", None, |c, _, _| match &c.trigger {
            Trigger::EnterBlock { blocks, state: props } => {
                blocks.as_ref().is_none_or(|b| block.is_some_and(|id| b.contains(id))) && props.as_ref().is_none_or(|p| state_matches(state, p))
            }
            _ => false,
        });
    }

    /// `ChangeDimensionTrigger.trigger`.
    pub(crate) fn changed_dimension(&mut self, from: &str, to: &str) {
        self.fire("minecraft:changed_dimension", None, |c, _, _| match &c.trigger {
            Trigger::ChangedDimension { from: f, to: t } => f.as_deref().is_none_or(|f| f == from) && t.as_deref().is_none_or(|t| t == to),
            _ => false,
        });
    }

    /// `TradeTrigger.trigger`.
    pub(crate) fn traded(&mut self, villager: &Subject, item: &ItemStack) {
        let loot = self.loot.clone();
        let loot_ref: &LootData = loot.as_deref().unwrap_or_else(|| empty_loot());
        let villager_ok = |cap: &criteria::Cap| {
            let ctx = TriggerCtx { this: villager, tags: &loot_ref.tags, origin: None, block: None, tool: None };
            criteria::test_cap(loot_ref, cap, &ctx)
        };
        self.fire("minecraft:villager_trade", None, |c, loot, _| match &c.trigger {
            Trigger::VillagerTrade { villager: v, item: p } => {
                v.as_ref().is_none_or(&villager_ok) && p.as_ref().is_none_or(|p| kiln_loot::predicate::item_matches(&loot.tags, p, item))
            }
            _ => false,
        });
    }

    /// `EnchantedItemTrigger.trigger`.
    pub(crate) fn enchanted_item(&mut self, item: &ItemStack, levels: i32) {
        self.fire("minecraft:enchanted_item", None, |c, loot, _| match &c.trigger {
            Trigger::EnchantedItem { item: p, levels: l } => {
                p.as_ref().is_none_or(|p| kiln_loot::predicate::item_matches(&loot.tags, p, item)) && kiln_loot::predicate::item::int_bounds(l, levels)
            }
            _ => false,
        });
    }

    /// `KilledTrigger.trigger` for `player_killed_entity` (the victim) or
    /// `entity_killed_player` (the killer); `damage_type` is the killing blow's type.
    pub(crate) fn killed(&mut self, trigger: &str, entity: &Subject, damage_type: &str, direct: bool) {
        let loot = self.loot.clone();
        let loot_ref: &LootData = loot.as_deref().unwrap_or_else(|| empty_loot());
        let entity_ok = |cap: &criteria::Cap| {
            let ctx = TriggerCtx { this: entity, tags: &loot_ref.tags, origin: None, block: None, tool: None };
            criteria::test_cap(loot_ref, cap, &ctx)
        };
        let type_id = kiln_data::synced_id("minecraft:damage_type", damage_type).unwrap_or(0);
        self.fire(trigger, None, |c, _, _| match &c.trigger {
            Trigger::Killed { entity: e, killing_blow } => {
                e.as_ref().is_none_or(&entity_ok)
                    && killing_blow.as_ref().is_none_or(|p| {
                        p.tags.iter().all(|t| crate::health::damage_type_tag(type_id, t.tag.as_str()) == t.expected)
                            && p.direct_entity.is_none()
                            && p.source_entity.is_none()
                            && p.is_direct.is_none_or(|d| d == direct)
                    })
            }
            _ => false,
        });
    }

    /// `EntityHurtPlayerTrigger.trigger` (`dealt`: before armor and effects, `taken`: after).
    pub(crate) fn hurt_trigger(&mut self, trigger: &str, entity: Option<&Subject>, dealt: f32, taken: f32, damage_type: &str) {
        let loot = self.loot.clone();
        let loot_ref: &LootData = loot.as_deref().unwrap_or_else(|| empty_loot());
        let entity_ok = |cap: &criteria::Cap| {
            entity.is_some_and(|e| {
                let ctx = TriggerCtx { this: e, tags: &loot_ref.tags, origin: None, block: None, tool: None };
                criteria::test_cap(loot_ref, cap, &ctx)
            })
        };
        let type_id = kiln_data::synced_id("minecraft:damage_type", damage_type).unwrap_or(0);
        let within = |b: &kiln_item::component::DoubleBounds, v: f32| b.min.is_none_or(|m| m <= v as f64) && b.max.is_none_or(|m| v as f64 <= m);
        self.fire(trigger, None, |c, _, _| match &c.trigger {
            Trigger::Hurt { dealt: d, taken: t, blocked, source, entity: e } => {
                within(d, dealt)
                    && within(t, taken)
                    && blocked.is_none_or(|b| !b)
                    && e.as_ref().is_none_or(&entity_ok)
                    && source.as_ref().is_none_or(|p| {
                        p.tags.iter().all(|t| crate::health::damage_type_tag(type_id, t.tag.as_str()) == t.expected)
                            && p.direct_entity.is_none()
                            && p.source_entity.is_none()
                    })
            }
            _ => false,
        });
    }

    /// `PlayerHurtEntityTrigger.trigger`: the player (the damage's direct entity when `direct`)
    /// hurt `victim`.
    pub(crate) fn player_hurt_entity(&mut self, victim: &Subject, dealt: f32, taken: f32, damage_type: &str, direct: bool) {
        let loot = self.loot.clone();
        let loot_ref: &LootData = loot.as_deref().unwrap_or_else(|| empty_loot());
        let origin = self.pos;
        let victim_ok = |cap: &criteria::Cap| {
            let ctx = TriggerCtx { this: victim, tags: &loot_ref.tags, origin: Some(origin), block: None, tool: None };
            criteria::test_cap(loot_ref, cap, &ctx)
        };
        let me = self.subject(None);
        let me_ok = |p: &kiln_loot::predicate::world::EntityPredicate| me.matches(&loot_ref.tags, p, origin);
        let type_id = kiln_data::synced_id("minecraft:damage_type", damage_type).unwrap_or(0);
        let within = |b: &kiln_item::component::DoubleBounds, v: f32| b.min.is_none_or(|m| m <= v as f64) && b.max.is_none_or(|m| v as f64 <= m);
        let hits: bool = {
            let data = self.advancements.data.clone();
            data.criteria_for("minecraft:player_hurt_entity").iter().any(|&(i, c)| self.advancements.listening(i, c))
        };
        if !hits {
            return;
        }
        let pass = |c: &Criterion| match &c.trigger {
            Trigger::Hurt { dealt: d, taken: t, blocked, source, entity: e } => {
                within(d, dealt)
                    && within(t, taken)
                    && blocked.is_none_or(|b| !b)
                    && e.as_ref().is_none_or(&victim_ok)
                    && source.as_ref().is_none_or(|p| {
                        p.tags.iter().all(|t| crate::health::damage_type_tag(type_id, t.tag.as_str()) == t.expected)
                            && p.direct_entity.as_ref().is_none_or(|d| direct && me_ok(d))
                            && p.source_entity.as_ref().is_none_or(|s| me_ok(s))
                            && p.is_direct.is_none_or(|d| d == direct)
                    })
            }
            _ => false,
        };
        let data = self.advancements.data.clone();
        let hits: Vec<(usize, usize)> =
            data.criteria_for("minecraft:player_hurt_entity").iter().copied().filter(|&(i, c)| self.advancements.listening(i, c) && pass(&data.list[i].criteria[c].1)).collect();
        drop(me);
        let now = now_millis();
        for (i, c) in hits {
            let crit = &data.list[i].criteria[c].1;
            if crit.player.as_ref().is_some_and(|cap| {
                let s = self.subject(None);
                let ctx = TriggerCtx { this: &s, tags: &loot_ref.tags, origin: None, block: None, tool: None };
                !criteria::test_cap(loot_ref, cap, &ctx)
            }) {
                continue;
            }
            self.advancements.award(i, c, now);
        }
    }

    /// `StartRidingTrigger.trigger`.
    pub(crate) fn started_riding(&mut self) {
        self.fire("minecraft:started_riding", None, |c, _, _| matches!(c.trigger, Trigger::Player));
    }

    /// Fires `trigger` for the criteria whose conditions ([`Conds`]) pass `test`, which gets
    /// a tester for entity conditions (`EntityPredicate.createContext`: the entity as `this`,
    /// the player's position as the origin).
    pub(crate) fn fire_conds(
        &mut self,
        trigger: &str,
        world: Option<&dyn WorldProbe>,
        test: impl Fn(&Conds, &dyn Fn(&criteria::Cap, &Subject) -> bool, &LootData) -> bool,
    ) {
        let origin = self.pos;
        self.fire(trigger, world, |c, loot, _| {
            let Trigger::Conds(conds) = &c.trigger else { return false };
            let ok = |cap: &criteria::Cap, s: &Subject| {
                let ctx = TriggerCtx { this: s, tags: &loot.tags, origin: Some(origin), block: None, tool: None };
                criteria::test_cap(loot, cap, &ctx)
            };
            test(conds, &ok, loot)
        });
    }

    /// `DistanceTrigger.trigger` (`fall_from_height`, `nether_travel`, `ride_entity_in_lava`):
    /// `start_position` tested where the trip began, `distance` from there to here.
    pub(crate) fn distance_trigger(&mut self, trigger: &str, start: [f64; 3]) {
        let (pos, dim) = (self.pos, crate::DIMENSIONS[self.dim].0);
        self.fire_conds(trigger, None, |c, _, _| {
            c.location("start_position").is_none_or(|l| criteria::location_matches(l, start, dim, None)) && c.distance("distance", start, pos)
        });
    }

    /// The trigger of a criterion event from the entity simulation.
    pub(crate) fn entity_criterion(&mut self, dim: &'static str, c: &kiln_entity::level::Criterion) {
        use kiln_entity::level::Criterion as E;
        let s = |seen: &kiln_entity::level::Seen| seen_subject(seen, dim);
        match c {
            E::BredAnimals { parent, partner, child } => {
                self.award_stat(*crate::player_stats::stat::ANIMALS_BRED, 1);
                let (a, b, ch) = (s(parent), s(partner), child.as_ref().map(s));
                // `BredAnimalsTrigger.TriggerInstance.matches`: the child, then the parents
                // either way round.
                self.fire_conds("minecraft:bred_animals", None, |c, ok, _| {
                    if let Some(cc) = c.cap("child")
                        && !ch.as_ref().is_some_and(|x| ok(cc, x))
                    {
                        return false;
                    }
                    let is = |key: &str, x: &Subject| c.cap(key).is_none_or(|cap| ok(cap, x));
                    (is("parent", &a) && is("partner", &b)) || (is("parent", &b) && is("partner", &a))
                });
            }
            E::TameAnimal { animal } => {
                let a = s(animal);
                self.fire_conds("minecraft:tame_animal", None, |c, ok, _| c.cap("entity").is_none_or(|cap| ok(cap, &a)));
            }
            E::SummonedEntity { entity } => {
                let a = s(entity);
                self.fire_conds("minecraft:summoned_entity", None, |c, ok, _| c.cap("entity").is_none_or(|cap| ok(cap, &a)));
            }
            E::CuredZombieVillager { zombie, villager } => {
                let (z, v) = (s(zombie), s(villager));
                self.fire_conds("minecraft:cured_zombie_villager", None, |c, ok, _| {
                    c.cap("zombie").is_none_or(|cap| ok(cap, &z)) && c.cap("villager").is_none_or(|cap| ok(cap, &v))
                });
            }
            E::LightningStrike { lightning, victims, .. } => {
                let l = s(lightning);
                let vs: Vec<Subject> = victims.iter().map(s).collect();
                self.fire_conds("minecraft:lightning_strike", None, |c, ok, _| {
                    c.cap("lightning").is_none_or(|cap| ok(cap, &l))
                        && c.cap_list("bystander").is_none_or(|b| b.iter().all(|cap| vs.iter().any(|v| ok(cap, v))))
                });
            }
            E::ChanneledLightning { victims } => {
                let vs: Vec<Subject> = victims.iter().map(s).collect();
                self.fire_conds("minecraft:channeled_lightning", None, |c, ok, _| {
                    c.cap_list("victims").is_none_or(|list| list.iter().all(|cap| vs.iter().any(|v| ok(cap, v))))
                });
            }
            E::KilledByArrow { victims, weapon } => {
                let vs: Vec<Subject> = victims.iter().map(s).collect();
                let types: std::collections::HashSet<i32> = vs.iter().map(|v| v.type_id).collect();
                self.fire_conds("minecraft:killed_by_arrow", None, |c, ok, loot| {
                    if let Some(p) = c.item("fired_from_weapon")
                        && !weapon.as_ref().is_some_and(|w| kiln_loot::predicate::item_matches(&loot.tags, p, w))
                    {
                        return false;
                    }
                    // Each condition takes a victim of its own.
                    let mut left: Vec<&Subject> = vs.iter().collect();
                    let all = c.cap_list("victims").is_none_or(|list| {
                        list.iter().all(|cap| match left.iter().position(|v| ok(cap, v)) {
                            Some(i) => {
                                left.remove(i);
                                true
                            }
                            None => false,
                        })
                    });
                    all && kiln_loot::predicate::item::int_bounds(&c.ints("unique_entity_types"), types.len() as i32)
                });
            }
            E::TargetHit { projectile, pos, signal } => {
                let p = s(projectile);
                let origin = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
                let _ = origin;
                self.fire_conds("minecraft:target_hit", None, |c, ok, _| {
                    kiln_loot::predicate::item::int_bounds(&c.ints("signal_strength"), *signal) && c.cap("projectile").is_none_or(|cap| ok(cap, &p))
                });
            }
        }
    }
}

use criteria::Conds;

/// `StatePropertiesPredicate` of `enter_block`: every named property has the value (or is in
/// the `{min, max}` range).
fn state_matches(state: u16, props: &kiln_loot::Json) -> bool {
    let Some(fields) = props.as_object() else { return false };
    fields.iter().all(|(name, want)| {
        let Some(have) = kiln_blocks::state::get(state, name) else { return false };
        match want {
            kiln_loot::Json::Obj(_) => {
                let (lo, hi) = (want.get("min").map(json_str), want.get("max").map(json_str));
                let num = |s: &str| s.parse::<i64>().ok();
                match (num(have), lo.as_deref().and_then(num), hi.as_deref().and_then(num)) {
                    (Some(v), l, h) => l.is_none_or(|l| v >= l) && h.is_none_or(|h| v <= h),
                    _ => lo.is_none_or(|l| l == have) && hi.is_none_or(|h| h == have),
                }
            }
            v => json_str(v) == have,
        }
    })
}

fn json_str(v: &kiln_loot::Json) -> String {
    match v {
        kiln_loot::Json::Str(s) => s.clone(),
        kiln_loot::Json::Bool(b) => b.to_string(),
        other => other.canonical(),
    }
}

//// A mob as criteria conditions see it.
pub(crate) fn mob_subject<'a>(e: &'a kiln_entity::Entity, dim: &'static str) -> Subject<'a> {
    seen_subject(&kiln_entity::level::Seen::of(e), dim)
}

/// An entity as it was when the event about it happened.
pub(crate) fn seen_subject(s: &kiln_entity::level::Seen, dim: &'static str) -> Subject<'static> {
    Subject {
        type_id: kiln_item::registry::ENTITY_TYPE.id(s.type_name).unwrap_or(-1),
        pos: [s.pos.x, s.pos.y, s.pos.z],
        dim,
        on_ground: s.on_ground,
        on_fire: s.on_fire,
        sneaking: false,
        sprinting: false,
        flying: false,
        baby: s.baby,
        equipment: Vec::new(),
        world: None,
        components: std::borrow::Cow::Owned(s.components.clone()),
        effects: Vec::new(),
        vehicle: None,
    }
}

impl Sim {
    /// The advancements of the enabled packs (on startup and `/reload`); players start over
    /// with their saved progress against the new tree.
    pub(crate) fn load_advancements(&mut self, roots: &[std::path::PathBuf]) {
        let refs: Vec<&std::path::Path> = roots.iter().map(std::path::PathBuf::as_path).collect();
        let data = Arc::new(super::Advancements::load(&refs, self.loot.as_deref()));
        self.advancements = data.clone();
        let mut conns: Vec<ConnId> = self.players.keys().copied().collect();
        conns.sort_unstable();
        for conn in conns {
            let json = self.players.get(&conn).map(|p| p.advancements.to_json());
            let Some(p) = self.players.get_mut(&conn) else { continue };
            let mut fresh = PlayerAdvancements::new(data.clone());
            if let Some(json) = json {
                let _ = fresh.load_json(&json);
            }
            p.advancements = fresh;
        }
    }

    /// `players/advancements/<uuid>.json`.
    fn advancements_path(&self, uuid: uuid::Uuid) -> Option<std::path::PathBuf> {
        self.storage.as_ref().map(|s| s.dir.join("players/advancements").join(format!("{}.json", uuid.hyphenated())))
    }

    /// A joining player's progress (`PlayerAdvancements.load`).
    pub(crate) fn load_player_advancements(&self, uuid: uuid::Uuid) -> PlayerAdvancements {
        let mut pa = PlayerAdvancements::new(self.advancements.clone());
        if let Some(path) = self.advancements_path(uuid)
            && let Ok(text) = std::fs::read_to_string(&path)
            && let Err(e) = pa.load_json(&text)
        {
            tracing::error!("Couldn't parse player advancements in {}: {e}", path.display());
        }
        pa
    }

    pub(crate) fn save_player_advancements(&self, p: &Player) {
        let Some(path) = self.advancements_path(p.uuid) else { return };
        let write = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, p.advancements.to_json()));
        if let Err(e) = write {
            tracing::error!("Couldn't save player advancements to {}: {e}", path.display());
        }
    }

    /// Serial upkeep for every player: completed advancements' rewards and announcements,
    /// then Update Advancements (`flushDirty`).
    pub(crate) fn advancement_upkeep(&mut self) {
        let mut conns: Vec<ConnId> = self.players.keys().copied().collect();
        conns.sort_unstable();
        for conn in conns {
            self.grant_completed(conn);
            if let Some(p) = self.players.get_mut(&conn)
                && let Some(pkt) = p.advancements.flush(true)
            {
                p.send(pkt);
            }
        }
    }

    /// What `PlayerAdvancements.award` does when an advancement completes: its rewards, then
    /// the announcement to everyone. Commands call this right away (vanilla announces before
    /// the command's feedback); region work leaves it to the next upkeep.
    pub(crate) fn grant_completed(&mut self, conn: ConnId) {
        let announce = self.rule_bool("minecraft:show_advancement_messages");
        let rules = self.rules.clone();
        let loot = self.loot.clone();
        let seed = self.config.noise.as_ref().map_or(0, |n| n.seed) ^ self.game_time;
        // Rewards can unlock recipes that complete more advancements.
        for _ in 0..64 {
            let Some(p) = self.players.get_mut(&conn) else { return };
            let done = std::mem::take(&mut p.advancements.completed.advancements);
            if done.is_empty() {
                break;
            }
            let data = p.advancements.data.clone();
            for i in done {
                let a = &data.list[i];
                let mut spawns = Vec::new();
                let Some(p) = self.players.get_mut(&conn) else { return };
                p.grant_rewards(a, &rules, loot.as_deref(), seed, &mut spawns);
                let (dim, name) = (p.dim, p.name.clone());
                self.dims[dim].spawns.extend(spawns);
                if let Some(f) = &a.rewards.function {
                    self.run_function_as_player(conn, f);
                }
                if announce
                    && a.display.as_ref().is_some_and(|d| d.announce_to_chat)
                    && let Some(text) = a.announcement(kiln_command::Text::literal(name))
                {
                    self.broadcast(kiln_proto::packets::system_chat(text.to_nbt(), false));
                    tracing::info!(target: "kiln_sim::commands", "{}", crate::commands::console_text(&text));
                }
            }
        }
    }

    /// `ServerFunctionManager.execute` as the player, output suppressed, permission level 2.
    fn run_function_as_player(&mut self, conn: ConnId, id: &str) {
        let Some(ident) = kiln_command::Identifier::parse(id) else { return };
        let Some(f) = self.commands.packs.library.get(&ident) else { return };
        let dispatcher = self.commands.dispatcher.clone();
        let source = crate::commands::CommandSource::Player(conn);
        let previous = std::mem::replace(&mut self.commands.source, source);
        let mut stack = self.source_stack(source);
        stack.max_permission = 2;
        stack.silent = true;
        let saved = std::mem::replace(&mut self.commands.stack, stack.clone());
        if let Err(e) = kiln_command::vanilla::run_as_server(&dispatcher, self, &f, stack) {
            tracing::warn!("Failed to execute function {id}: {}", e.to_plain());
        }
        self.commands.stack = saved;
        self.commands.source = previous;
        self.flush_scoreboard();
    }

}

impl Player {
    /// `AdvancementRewards.grant`: experience, loot (into the inventory, the rest dropped),
    /// recipes.
    fn grant_rewards(&mut self, a: &super::Advancement, rules: &kiln_inventory::Rules, loot: Option<&LootData>, seed: i64, spawns: &mut Vec<crate::entities::Spawn>) {
        let r = &a.rewards;
        if r.experience != 0 {
            self.give_experience_points(r.experience);
        }
        if let Some(loot) = loot {
            let ctx = RewardContext { pos: self.pos };
            for (k, table) in r.loot.iter().enumerate() {
                let s = crate::mobs::loot_seed(seed, self.entity_id as i64, k as i32, 0x6164_7672);
                for mut stack in crate::mobs::roll(loot, table, &ctx, s) {
                    self.add_to_inventory(&mut stack);
                    if !stack.is_empty() {
                        spawns.push(self.throw(stack));
                    }
                }
            }
        }
        if !r.recipes.is_empty() {
            self.award_recipes_by_key(rules, &r.recipes);
        }
    }
}

/// `LootContextParamSets.ADVANCEMENT_REWARD`: `this_entity` (the player) and `origin`.
struct RewardContext {
    pos: [f64; 3],
}

impl kiln_loot::LootContext for RewardContext {
    fn has_entity(&self, target: kiln_loot::EntityTarget) -> bool {
        target == kiln_loot::EntityTarget::This
    }
    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.pos)
    }
}
