//! Buckets (`BucketItem.use`, `MobBucketItem`, `SolidBucketItem`) and cauldrons
//! (`CauldronInteractions`).
//!
//! An empty bucket fills from the source block or waterlogged block the player looks at
//! (`BucketPickup.pickupBlock`: liquids, waterloggable blocks, powder snow, bubble columns); a
//! full one empties into the block it hits (a waterloggable block takes water in) or the one in
//! front of it, breaking what the liquid replaces, evaporating in the Nether. Fish, axolotl and
//! tadpole buckets also let their mob out. `filled_bucket` and `placed_block` fire.
//!
//! Cauldrons react to buckets, bottles, potions and washable items before the item's own use.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::Spawn;
use crate::player_stats::{self, Stat};
use crate::use_item::{FluidMode, pov_hit};
use kiln_blocks::{BlockPos, Direction, Effect, Level, flags};
use kiln_data::block_logic::{self as logic, BlockClass as C, FluidKind, interface};
use kiln_data::blocks::default_state as d;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;

/// What a bucket item holds (`BucketItem.content`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Content {
    Empty,
    Water,
    Lava,
    /// A mob bucket with no fluid (`MobBucketItem` of `Fluids.EMPTY`: the sulfur cube's).
    NoFluid,
}

/// A bucket item: its fluid, and the mob a mob bucket lets out (with its empty sound).
fn bucket(name: &str) -> Option<(Content, Option<(&'static str, &'static str)>)> {
    Some(match name {
        "minecraft:bucket" => (Content::Empty, None),
        "minecraft:water_bucket" => (Content::Water, None),
        "minecraft:lava_bucket" => (Content::Lava, None),
        "minecraft:pufferfish_bucket" => (Content::Water, Some(("minecraft:pufferfish", "minecraft:item.bucket.empty_fish"))),
        "minecraft:salmon_bucket" => (Content::Water, Some(("minecraft:salmon", "minecraft:item.bucket.empty_fish"))),
        "minecraft:cod_bucket" => (Content::Water, Some(("minecraft:cod", "minecraft:item.bucket.empty_fish"))),
        "minecraft:tropical_fish_bucket" => (Content::Water, Some(("minecraft:tropical_fish", "minecraft:item.bucket.empty_fish"))),
        "minecraft:axolotl_bucket" => (Content::Water, Some(("minecraft:axolotl", "minecraft:item.bucket.empty_axolotl"))),
        "minecraft:tadpole_bucket" => (Content::Water, Some(("minecraft:tadpole", "minecraft:item.bucket.empty_tadpole"))),
        "minecraft:sulfur_cube_bucket" => (Content::NoFluid, Some(("minecraft:sulfur_cube", "minecraft:item.bucket.empty_sulfur_cube"))),
        _ => return None,
    })
}

/// Whether `Item.use` of this item is `BucketItem.use`.
pub(crate) fn is_bucket(name: &str) -> bool {
    bucket(name).is_some()
}

/// `BucketItem.use` with the bucket in `off_hand` or the main hand. The caller checked
/// [`is_bucket`].
pub(crate) fn use_bucket(p: &mut Player, level: &mut RegionLevel, off_hand: bool, spawns: &mut Vec<Spawn>) {
    let held = p.in_hand(off_hand).clone();
    let Some((content, mob)) = bucket(held.item_name()) else { return };
    let fluid = if content == Content::Empty { FluidMode::SourceOnly } else { FluidMode::None };
    let hit = {
        let lv = &*level;
        pov_hit(p, &|pos| lv.block(pos), fluid)
    };
    let Some(hit) = hit else { return };
    let (pos, dir) = (hit.pos, hit.face);
    let next = pos.relative(dir);
    // `mayInteract` and `mayUseItemAt` (adventure mode cannot).
    if p.game_mode > 1 || !level.in_bounds(pos) {
        return;
    }
    if content == Content::Empty {
        let state = level.block(pos);
        let Some((filled, sound)) = pickup_block(level, pos, state) else { return };
        p.award_stat(Stat::item(player_stats::USED, held.item()), 1);
        p.queue_sound(sound, 1.0, 1.0);
        p.fill_in_hand(off_hand, filled.clone(), true, spawns);
        p.filled_bucket(&filled);
        return;
    }
    let state = level.block(pos);
    let target = if logic::implements(state, interface::LIQUID_BLOCK_CONTAINER) && content == Content::Water { pos } else { next };
    if !empty_contents(Some(&mut *p), level, content, mob.map(|m| m.1), target, Some((pos, dir))) {
        return;
    }
    // `MobBucketItem.checkExtraContent`: the mob comes out.
    if let Some((mob_type, _)) = mob
        && let Some(spawn) = release_mob(level, &held, mob_type, target)
    {
        spawns.push(spawn);
    }
    let probe = crate::advancements::triggers::CellProbe::new(&*level.cells, level.env);
    p.used_on_block("minecraft:placed_block", [target.x, target.y, target.z], level.block(target), &held, &probe);
    p.award_stat(Stat::item(player_stats::USED, held.item()), 1);
    // `getEmptySuccessItem`: an empty bucket (creative players keep theirs).
    if !p.infinite_materials() {
        let empty = ItemStack::of("minecraft:bucket", 1).unwrap_or_else(ItemStack::empty);
        p.fill_in_hand(off_hand, empty, true, spawns);
    }
}

/// `MobBucketItem.spawn`: the mob of a bucket at the block `target` (`EntitySpawnReason.BUCKET`, aligned to
/// the floor of the block), as the bucket kept it.
fn release_mob(level: &RegionLevel, bucket: &ItemStack, mob_type: &str, target: BlockPos) -> Option<Spawn> {
    use kiln_entity::mob::MobKind;
    let kind = MobKind::by_name(mob_type)?;
    let width = kiln_data::entities::by_name(kind.type_name())?.width;
    let off = crate::mobs::align_offset(level, target, width);
    let at = [target.x as f64 + 0.5, target.y as f64 + off, target.z as f64 + 0.5];
    let env = level.env;
    let seed = crate::mobs::loot_seed(env.seed, env.game_time, 0, (target.x as u64) << 32 ^ target.z as u64 ^ (target.y as u64) << 16 ^ 0x6275_636b);
    let mut spawn = match kind {
        // An axolotl keeps its variant, health and age in the bucket.
        MobKind::Axolotl => crate::mobs::bucket_axolotl(bucket, at)?,
        MobKind::Salmon | MobKind::Cod | MobKind::Pufferfish | MobKind::TropicalFish | MobKind::Tadpole => {
            let mut s = crate::mobs::bucket_release(bucket, [target.x, target.y, target.z], env.mobs.difficulty, env.game_time, seed)?;
            s.pos = at;
            if let crate::entities::Body::Ready(e) = &mut s.body {
                e.set_pos(kiln_entity::math::Vec3::new(at[0], at[1], at[2]));
                e.set_old_pos_and_rot();
            }
            s
        }
        _ => crate::mobs::spawn(kind, at, None, None),
    };
    spawn.pos = at;
    Some(spawn)
}

/// `DispenseItemBehavior$3` (`DispensibleContainerItem.emptyContents` with no one holding the bucket, then
/// `checkExtraContent`): a full bucket emptied at the block `pos`. Whether it was.
pub(crate) fn dispense_empty(level: &mut RegionLevel, stack: &ItemStack, pos: BlockPos) -> bool {
    let name = stack.item_name();
    if name == "minecraft:powder_snow_bucket" {
        // `SolidBucketItem.emptyContents`: only into an empty block.
        if level.in_bounds(pos) && kiln_data::blocks_types::is_air(level.block(pos)) {
            kiln_blocks::set_block_and_update(level, pos, d::POWDER_SNOW);
            level.effect(Effect::GameEvent { pos, event: "minecraft:block_place" });
            level.effect(Effect::ActorSound { pos, sound: "minecraft:item.bucket.empty_powder_snow", volume: 1.0, pitch: 1.0 });
            return true;
        }
        return false;
    }
    let Some((content, mob)) = bucket(name) else { return false };
    if content == Content::Empty || !empty_contents(None, level, content, mob.map(|m| m.1), pos, None) {
        return false;
    }
    if let Some((mob_type, _)) = mob
        && let Some(spawn) = release_mob(level, stack, mob_type, pos)
    {
        level.out.spawns.push(spawn);
        level.effect(Effect::GameEvent { pos, event: "minecraft:entity_place" });
    }
    true
}

/// `BucketPickup.pickupBlock`: the filled bucket and its sound, the block drained.
fn pickup_block(level: &mut RegionLevel, pos: BlockPos, state: u16) -> Option<(ItemStack, &'static str)> {
    let of = |n: &str| ItemStack::of(n, 1);
    if !logic::implements(state, interface::BUCKET_PICKUP) {
        return None;
    }
    match logic::block_class(state) {
        C::LiquidBlock => {
            if kiln_blocks::state::get_int(state, "level") != 0 {
                return None;
            }
            let lava = logic::fluid(state).kind == FluidKind::Lava;
            kiln_blocks::set_block(level, pos, d::AIR, flags::ALL_IMMEDIATE);
            if lava {
                Some((of("minecraft:lava_bucket")?, "minecraft:item.bucket.fill_lava"))
            } else {
                Some((of("minecraft:water_bucket")?, "minecraft:item.bucket.fill"))
            }
        }
        C::PowderSnowBlock => {
            kiln_blocks::set_block(level, pos, d::AIR, flags::ALL_IMMEDIATE);
            level.effect(Effect::LevelEvent { id: 2001, pos, data: state as i32 });
            Some((of("minecraft:powder_snow_bucket")?, "minecraft:item.bucket.fill_powder_snow"))
        }
        C::BubbleColumnBlock => {
            kiln_blocks::set_block(level, pos, d::AIR, flags::ALL_IMMEDIATE);
            Some((of("minecraft:water_bucket")?, "minecraft:item.bucket.fill"))
        }
        _ if logic::implements(state, interface::SIMPLE_WATERLOGGED_BLOCK) => {
            if !kiln_blocks::state::get_bool(state, "waterlogged") {
                return None;
            }
            let drained = kiln_blocks::state::set_bool(state, "waterlogged", false);
            kiln_blocks::set_block_and_update(level, pos, drained);
            if !kiln_blocks::behaviour::can_survive(level, drained, pos) {
                kiln_blocks::destroy_block(level, pos, true, flags::LIMIT);
            }
            Some((of("minecraft:water_bucket")?, "minecraft:item.bucket.fill"))
        }
        _ => None,
    }
}

/// `BucketItem.emptyContents`: pours the bucket's fluid at `pos` (see the module docs);
/// `hit` retries in front of the hit face when `pos` cannot take it.
fn empty_contents(
    mut p: Option<&mut Player>,
    level: &mut RegionLevel,
    content: Content,
    mob_sound: Option<&'static str>,
    pos: BlockPos,
    hit: Option<(BlockPos, Direction)>,
) -> bool {
    let (kind, fluid_type) = match content {
        Content::Water => (FluidKind::Water, kiln_blocks::FluidType::Water),
        Content::Lava => (FluidKind::Lava, kiln_blocks::FluidType::Lava),
        Content::Empty => return false,
        // `MobBucketItem.emptyContents` of a bucket with no fluid: just the sound.
        Content::NoFluid => {
            play_empty_sound(level, pos, content, mob_sound);
            return true;
        }
    };
    let state = level.block(pos);
    // `BlockState.canBeReplaced(Fluid)`: replaceable or not solid.
    let replaceable = kiln_entity::physics::can_be_replaced(state) || !kiln_entity::physics::is_solid(state);
    let container = logic::implements(state, interface::LIQUID_BLOCK_CONTAINER);
    let placeable = replaceable || container && kiln_blocks::fluid::can_place_liquid(state, fluid_type);
    let air = kiln_data::blocks_types::is_air(state);
    let sneaking = p.as_ref().is_some_and(|p| p.sneaking);
    if !(air || placeable && (!sneaking || hit.is_none())) {
        return match hit {
            Some((at, dir)) => empty_contents(p, level, content, mob_sound, at.relative(dir), None),
            None => false,
        };
    }
    if level.rules().water_evaporates && kind == FluidKind::Water {
        let r = level.random();
        let pitch = 2.6 + (r.next_float() - r.next_float()) * 0.8;
        level.effect(Effect::ActorSound { pos, sound: "minecraft:block.fire.extinguish", volume: 0.5, pitch });
        if let Some(smoke) = kiln_data::builtin_id("minecraft:particle_type", "minecraft:large_smoke")
            && let Some(p) = p.as_deref_mut()
        {
            use kiln_proto::packets::world_fx;
            p.send(world_fx::level_particles(&world_fx::LevelParticles {
                particle: world_fx::Particle { kind: smoke, options: world_fx::ParticleOptions::None },
                override_limiter: false,
                always_show: false,
                pos: [pos.x as f64, pos.y as f64, pos.z as f64],
                offset: [1.0, 1.0, 1.0],
                max_speed: [0.0, 0.0, 0.0],
                count: 8,
                randomization: world_fx::ParticleRandomization::Alternative,
            }));
        }
        return true;
    }
    if container && content == Content::Water {
        let source = kiln_data::block_logic::Fluid { kind, source: true, falling: false, amount: 8 };
        kiln_blocks::fluid::place_liquid(level, pos, state, source);
        play_empty_sound(level, pos, content, mob_sound);
        return true;
    }
    // What the liquid replaces breaks with its drops (a liquid is simply replaced).
    if replaceable && !kiln_entity::physics::is_liquid(state) {
        kiln_blocks::destroy_block(level, pos, true, flags::LIMIT);
    }
    let block = if kind == FluidKind::Water { d::WATER } else { d::LAVA };
    let set = kiln_blocks::set_block(level, pos, block, flags::ALL_IMMEDIATE);
    if set || logic::fluid(state).source {
        play_empty_sound(level, pos, content, mob_sound);
        return true;
    }
    false
}

/// `BucketItem.playEmptySound` (`MobBucketItem`: its mob's sound).
fn play_empty_sound(level: &mut RegionLevel, pos: BlockPos, content: Content, mob_sound: Option<&'static str>) {
    let sound = match (mob_sound, content) {
        (Some(s), _) => s,
        (None, Content::Lava) => "minecraft:item.bucket.empty_lava",
        _ => "minecraft:item.bucket.empty",
    };
    level.effect(Effect::ActorSound { pos, sound, volume: 1.0, pitch: 1.0 });
}

/// The cauldron at `pos` handles the item in the hand (`AbstractCauldronBlock.useItemOn`):
/// `Some(true)` when an interaction took place, `Some(false)` for a cauldron that had nothing
/// for it (`TRY_WITH_EMPTY_HAND`), `None` when `pos` is no cauldron.
pub(crate) fn use_cauldron(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, off_hand: bool, spawns: &mut Vec<Spawn>) -> Option<bool> {
    let state = level.block(pos);
    if !logic::is_instance(state, C::AbstractCauldronBlock) {
        return None;
    }
    let held = p.in_hand(off_hand).clone();
    if held.is_empty() {
        return Some(false);
    }
    let name = held.item_name();
    let class = logic::block_class(state);
    let water = kiln_blocks::state::same_block(state, d::WATER_CAULDRON);
    let lava = class == C::LavaCauldronBlock;
    let snow = kiln_blocks::state::same_block(state, d::POWDER_SNOW_CAULDRON);
    let full = kiln_blocks::state::has(state, "level") && kiln_blocks::state::get_int(state, "level") == 3;
    let bucket = || ItemStack::of("minecraft:bucket", 1).unwrap_or_else(ItemStack::empty);
    let under_water = || logic::fluid(level.block(pos.above())).kind == FluidKind::Water;
    let used = Stat::item(player_stats::USED, held.item());
    let potion_is_water = || {
        held.get(kiln_item::keys::POTION_CONTENTS)
            .is_some_and(|c| c.potion.is_some_and(|id| kiln_item::registry::POTION.name(id) == Some("minecraft:water")))
    };
    // `addDefaultInteractions` of every dispatcher: buckets pour into the cauldron.
    let pour = |to: u16, sound: &'static str| (to, sound);
    let poured = match name {
        "minecraft:water_bucket" => Some(pour(kiln_blocks::state::set_int(d::WATER_CAULDRON, "level", 3), "minecraft:item.bucket.empty")),
        "minecraft:lava_bucket" if !under_water() => Some(pour(d::LAVA_CAULDRON, "minecraft:item.bucket.empty_lava")),
        "minecraft:powder_snow_bucket" if !under_water() => {
            Some(pour(kiln_blocks::state::set_int(d::POWDER_SNOW_CAULDRON, "level", 3), "minecraft:item.bucket.empty_powder_snow"))
        }
        // Under water, lava and powder snow do nothing but still count as handled (`CONSUME`).
        "minecraft:lava_bucket" | "minecraft:powder_snow_bucket" => return Some(true),
        _ => None,
    };
    if let Some((to, sound)) = poured {
        // `emptyBucket`.
        p.fill_in_hand(off_hand, bucket(), true, spawns);
        p.award_stat(player_stats::custom("minecraft:fill_cauldron"), 1);
        p.award_stat(used, 1);
        kiln_blocks::set_block_and_update(level, pos, to);
        level.effect(Effect::Sound { pos, sound, volume: 1.0, pitch: 1.0 });
        return Some(true);
    }
    // `fillBucket`: a full cauldron into an empty bucket.
    if name == "minecraft:bucket" {
        let (filled, sound) = if water && full {
            ("minecraft:water_bucket", "minecraft:item.bucket.fill")
        } else if lava {
            ("minecraft:lava_bucket", "minecraft:item.bucket.fill_lava")
        } else if snow && full {
            ("minecraft:powder_snow_bucket", "minecraft:item.bucket.fill_powder_snow")
        } else {
            return Some(false);
        };
        let filled = ItemStack::of(filled, 1).unwrap_or_else(ItemStack::empty);
        p.fill_in_hand(off_hand, filled, true, spawns);
        p.award_stat(player_stats::custom("minecraft:use_cauldron"), 1);
        p.award_stat(used, 1);
        kiln_blocks::set_block_and_update(level, pos, d::CAULDRON);
        level.effect(Effect::Sound { pos, sound, volume: 1.0, pitch: 1.0 });
        return Some(true);
    }
    let bottle = || ItemStack::of("minecraft:glass_bottle", 1).unwrap_or_else(ItemStack::empty);
    // An empty cauldron takes a water bottle.
    if class == C::CauldronBlock {
        if name != "minecraft:potion" || !potion_is_water() {
            return Some(false);
        }
        p.fill_in_hand(off_hand, bottle(), true, spawns);
        p.award_stat(player_stats::custom("minecraft:use_cauldron"), 1);
        p.award_stat(used, 1);
        kiln_blocks::set_block_and_update(level, pos, d::WATER_CAULDRON);
        level.effect(Effect::Sound { pos, sound: "minecraft:item.bottle.empty", volume: 1.0, pitch: 1.0 });
        return Some(true);
    }
    if !water {
        return Some(false);
    }
    let level_now = kiln_blocks::state::get_int(state, "level");
    match name {
        "minecraft:glass_bottle" => {
            let mut potion = ItemStack::of("minecraft:potion", 1).unwrap_or_else(ItemStack::empty);
            potion.insert(
                kiln_item::keys::POTION_CONTENTS,
                kiln_item::component::PotionContents { potion: kiln_item::registry::POTION.id("minecraft:water"), ..Default::default() },
            );
            p.fill_in_hand(off_hand, potion, true, spawns);
            p.award_stat(player_stats::custom("minecraft:use_cauldron"), 1);
            p.award_stat(used, 1);
            lower_fill_level(level, pos, state);
            level.effect(Effect::Sound { pos, sound: "minecraft:item.bottle.fill", volume: 1.0, pitch: 1.0 });
            Some(true)
        }
        "minecraft:potion" => {
            if level_now == 3 || !potion_is_water() {
                return Some(false);
            }
            p.fill_in_hand(off_hand, bottle(), true, spawns);
            // (Unlike an empty cauldron's, this branch counts no use of the item: only the cauldron's.)
            p.award_stat(player_stats::custom("minecraft:use_cauldron"), 1);
            kiln_blocks::set_block_and_update(level, pos, kiln_blocks::state::set_int(state, "level", level_now + 1));
            level.effect(Effect::Sound { pos, sound: "minecraft:item.bottle.empty", volume: 1.0, pitch: 1.0 });
            Some(true)
        }
        _ if name.ends_with("_banner") => {
            // `bannerInteraction`: the last pattern layer washes off one banner.
            let Some(layers) = held.get(kiln_item::keys::BANNER_PATTERNS) else { return Some(false) };
            if layers.0.is_empty() {
                return Some(false);
            }
            let mut washed = held.with_count(1);
            let mut rest = layers.clone();
            rest.0.pop();
            washed.insert(kiln_item::keys::BANNER_PATTERNS, rest);
            p.fill_in_hand(off_hand, washed, false, spawns);
            p.award_stat(player_stats::custom("minecraft:clean_banner"), 1);
            lower_fill_level(level, pos, state);
            Some(true)
        }
        _ if name.ends_with("_shulker_box") => {
            // `shulkerBoxInteraction`: a dyed shulker box becomes a plain one.
            let Some(plain) = ItemStack::of("minecraft:shulker_box", 1) else { return Some(false) };
            let washed = ItemStack::from_parts(plain.item(), 1, held.patch().clone());
            p.fill_in_hand(off_hand, washed, false, spawns);
            p.award_stat(player_stats::custom("minecraft:clean_shulker_box"), 1);
            lower_fill_level(level, pos, state);
            Some(true)
        }
        _ if kiln_inventory::tags::contains("minecraft:item", "minecraft:cauldron_can_remove_dye", held.item()) => {
            // `dyedItemIteration`: the dye washes off.
            if !held.has(kiln_item::component::ids::DYED_COLOR) {
                return Some(false);
            }
            let i = p.hand_index(off_hand);
            kiln_inventory::Container::item_mut(&mut p.inv, i).remove(kiln_item::component::ids::DYED_COLOR);
            p.inv.times_changed += 1;
            p.award_stat(player_stats::custom("minecraft:clean_armor"), 1);
            lower_fill_level(level, pos, state);
            Some(true)
        }
        _ => Some(false),
    }
}

/// `LayeredCauldronBlock.lowerFillLevel`.
fn lower_fill_level(level: &mut RegionLevel, pos: BlockPos, state: u16) {
    let n = kiln_blocks::state::get_int(state, "level") - 1;
    let to = if n == 0 { d::CAULDRON } else { kiln_blocks::state::set_int(state, "level", n) };
    kiln_blocks::set_block_and_update(level, pos, to);
}

impl Player {
    /// `FilledBucketTrigger.trigger`.
    pub(crate) fn filled_bucket(&mut self, filled: &ItemStack) {
        self.fire_conds("minecraft:filled_bucket", None, |c, _, loot| {
            c.item("item").is_none_or(|ip| kiln_loot::predicate::item_matches(&loot.tags, ip, filled))
        });
    }
}
