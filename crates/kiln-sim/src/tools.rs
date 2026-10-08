//! Tools and other items used on blocks (`Item.useOn`, and `useItemOn` of the blocks that react
//! to an item): hoes till (`HoeItem.TILLABLES`), shovels flatten paths and put out campfires
//! (`ShovelItem`), axes strip logs, scrape copper and take wax off (`AxeItem`), honeycomb waxes
//! copper (`HoneycombItem`), shears trim growing plant heads and carve pumpkins, bone meal
//! grows crops, stems, berries, cocoa, tall flowers, and (in kiln-blocks, with the level's
//! worldgen) saplings into trees, azaleas, huge mushrooms, grass and its flowers
//! (`BoneMealItem`), flint and steel and fire charges light campfires and candles, fire
//! charges set fire, and composters take compostable items and give bone meal
//! (`ComposterBlock`).

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::Spawn;
use crate::player_stats::{self, Stat};
use kiln_blocks::{BlockId, BlockPos, Direction, Effect, Level, flags, state};
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_data::blocks::default_state as d;
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;
use kiln_javamath::random::RandomSource;

fn short(state: u16) -> &'static str {
    let name = BlockId::of(state).name();
    name.strip_prefix("minecraft:").unwrap_or(name)
}

/// The default state of `minecraft:<name>` with `from`'s properties, if the block exists.
fn same_shape(name: &str, from: u16) -> Option<u16> {
    let id = BlockId::by_name(&format!("minecraft:{name}"))?;
    Some(state::with_properties_of(id.default_state(), from))
}

/// `AxeItem.getStripped`: logs, woods, stems, hyphae and bamboo blocks.
fn stripped(s: u16) -> Option<u16> {
    let n = short(s);
    if n.starts_with("stripped_") {
        return None;
    }
    let strippable = n.ends_with("_log") || n.ends_with("_wood") || n.ends_with("_stem") || n.ends_with("_hyphae") || n == "bamboo_block";
    if !strippable {
        return None;
    }
    same_shape(&format!("stripped_{n}"), s)
}

/// `WeatheringCopper.getPrevious`: one oxidation stage back.
fn scraped(s: u16) -> Option<u16> {
    let n = short(s);
    if n.starts_with("waxed_") {
        return None;
    }
    for (stage, previous) in [("exposed_", ""), ("weathered_", "exposed_"), ("oxidized_", "weathered_")] {
        if let Some(rest) = n.strip_prefix(stage) {
            let base = if previous.is_empty() && rest == "copper" { "copper_block".to_owned() } else { format!("{previous}{rest}") };
            return same_shape(&base, s);
        }
    }
    None
}

/// `HoneycombItem.WAX_OFF_BY_BLOCK`.
fn unwaxed(s: u16) -> Option<u16> {
    let rest = short(s).strip_prefix("waxed_")?;
    same_shape(rest, s)
}

/// `HoneycombItem.getWaxed`: copper blocks that have a waxed form.
fn waxed(s: u16) -> Option<u16> {
    let n = short(s);
    if n.starts_with("waxed_") || !(n.contains("copper") || n.contains("lightning_rod")) {
        return None;
    }
    same_shape(&format!("waxed_{n}"), s)
}

pub(crate) fn hand_slot(off_hand: bool) -> EquipmentSlot {
    if off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand }
}

/// What `useOn` did: nothing (`PASS`), or something (`SUCCESS`: the item was used).
pub(crate) fn item_use_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, face: Direction, off_hand: bool, spawns: &mut Vec<Spawn>) -> bool {
    let stack = p.in_hand(off_hand).clone();
    if stack.is_empty() {
        return false;
    }
    let name = stack.item_name();
    let s = level.block(pos);
    let used = if name.ends_with("_hoe") {
        till(p, level, pos, face, s, spawns)
    } else if name.ends_with("_shovel") {
        flatten(p, level, pos, face, s)
    } else if name.ends_with("_axe") {
        axe(p, level, pos, s, off_hand)
    } else if name == "minecraft:honeycomb" {
        wax(p, level, pos, s, off_hand)
    } else if name == "minecraft:shears" {
        trim(level, pos, s)
    } else if name == "minecraft:bone_meal" {
        bone_meal(p, level, pos, face, off_hand, spawns)
    } else if name == "minecraft:fire_charge" {
        fire_charge(p, level, pos, face, s, off_hand)
    } else if name == "minecraft:flint_and_steel" {
        light(p, level, pos, s, "minecraft:item.flintandsteel.use")
    } else {
        return false;
    };
    if !used {
        return false;
    }
    // Tools wear by one use (`hurtAndBreak(1)`); honeycomb, bone meal and fire charges are
    // used up where they are handled.
    if name.ends_with("_hoe") || name.ends_with("_shovel") || name.ends_with("_axe") || name == "minecraft:shears" || name == "minecraft:flint_and_steel" {
        p.hurt_and_break(hand_slot(off_hand), 1, None);
    }
    // `ItemStack.useOn` and `ServerPlayerGameMode.useItemOn`.
    p.award_stat(Stat::item(player_stats::USED, stack.item()), 1);
    let probe = crate::advancements::triggers::CellProbe::new(&*level.cells, level.env);
    p.used_on_block("minecraft:item_used_on_block", [pos.x, pos.y, pos.z], level.block(pos), &stack, &probe);
    true
}

/// `HoeItem.useOn`.
fn till(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, face: Direction, s: u16, spawns: &mut Vec<Spawn>) -> bool {
    let air_above = face != Direction::Down && kiln_data::blocks_types::is_air(level.block(pos.above()));
    let (to, drop) = match short(s) {
        "grass_block" | "dirt_path" | "dirt" if air_above => (d::FARMLAND, None),
        "coarse_dirt" if air_above => (d::DIRT, None),
        "rooted_dirt" => (d::DIRT, Some("minecraft:hanging_roots")),
        _ => return false,
    };
    level.effect(Effect::ActorSound { pos, sound: "minecraft:item.hoe.till", volume: 1.0, pitch: 1.0 });
    kiln_blocks::set_block(level, pos, to, flags::ALL_IMMEDIATE);
    // `Block.popResourceFromFace`.
    if let Some(item) = drop.and_then(|n| ItemStack::of(n, 1)) {
        let step = face.step();
        let at = [
            pos.x as f64 + 0.5 + if step[0] == 0 { level.random().next_double() * 0.5 - 0.25 } else { step[0] as f64 * 0.5 + step[0] as f64 * 0.125 },
            pos.y as f64 + 0.5 + if step[1] == 0 { level.random().next_double() * 0.5 - 0.25 } else { step[1] as f64 * 0.5 + step[1] as f64 * 0.125 } - 0.125,
            pos.z as f64 + 0.5 + if step[2] == 0 { level.random().next_double() * 0.5 - 0.25 } else { step[2] as f64 * 0.5 + step[2] as f64 * 0.125 },
        ];
        spawns.push(crate::mobs::drop_item(item, at, (pos.x as u64) << 20 ^ pos.z as u64 ^ p.entity_id as u64));
    }
    true
}

/// `ShovelItem.useOn`.
fn flatten(_p: &mut Player, level: &mut RegionLevel, pos: BlockPos, face: Direction, s: u16) -> bool {
    if face == Direction::Down {
        return false;
    }
    let flattenable = matches!(short(s), "grass_block" | "dirt" | "podzol" | "coarse_dirt" | "mycelium" | "rooted_dirt");
    let to = if flattenable && kiln_data::blocks_types::is_air(level.block(pos.above())) {
        level.effect(Effect::ActorSound { pos, sound: "minecraft:item.shovel.flatten", volume: 1.0, pitch: 1.0 });
        d::DIRT_PATH
    } else if logic::is_instance(s, C::CampfireBlock) && state::get_bool(s, "lit") {
        level.effect(Effect::ActorLevelEvent { id: 1009, pos, data: 0 });
        state::set_bool(s, "lit", false)
    } else {
        return false;
    };
    kiln_blocks::set_block(level, pos, to, flags::ALL_IMMEDIATE);
    true
}

/// `AxeItem.useOn`: strip, scrape or take the wax off (not while a shield in the other hand is
/// about to be raised).
fn axe(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, off_hand: bool) -> bool {
    if !off_hand && p.inv.equipped(EquipmentSlot::OffHand).get(kiln_item::keys::BLOCKS_ATTACKS).is_some() && !p.sneaking {
        return false;
    }
    let to = if let Some(to) = stripped(s) {
        level.effect(Effect::ActorSound { pos, sound: "minecraft:item.axe.strip", volume: 1.0, pitch: 1.0 });
        to
    } else if let Some(to) = scraped(s) {
        level.effect(Effect::ActorSound { pos, sound: "minecraft:item.axe.scrape", volume: 1.0, pitch: 1.0 });
        level.effect(Effect::ActorLevelEvent { id: 3005, pos, data: 0 });
        to
    } else if let Some(to) = unwaxed(s) {
        level.effect(Effect::ActorSound { pos, sound: "minecraft:item.axe.wax_off", volume: 1.0, pitch: 1.0 });
        level.effect(Effect::ActorLevelEvent { id: 3004, pos, data: 0 });
        to
    } else {
        return false;
    };
    kiln_blocks::set_block(level, pos, to, flags::ALL_IMMEDIATE);
    // Doors and tall blocks change both halves through their shape updates.
    true
}

/// `HoneycombItem.useOn` (the honeycomb is used up, creative players' too).
fn wax(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, off_hand: bool) -> bool {
    let Some(to) = waxed(s) else { return false };
    let i = p.hand_index(off_hand);
    kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
    p.inv.times_changed += 1;
    kiln_blocks::set_block(level, pos, to, flags::ALL_IMMEDIATE);
    level.effect(Effect::ActorLevelEvent { id: 3003, pos, data: 0 });
    true
}

/// `ShearsItem.useOn`: a growing plant's head stops growing.
fn trim(level: &mut RegionLevel, pos: BlockPos, s: u16) -> bool {
    if !logic::is_instance(s, C::GrowingPlantHeadBlock) || state::get_int(s, "age") >= 25 {
        return false;
    }
    level.effect(Effect::ActorSound { pos, sound: "minecraft:block.growing_plant.crop", volume: 1.0, pitch: 1.0 });
    kiln_blocks::set_block_and_update(level, pos, state::set_int(s, "age", 25));
    true
}

/// `CampfireBlock`/`CandleBlock`/`CandleCakeBlock.canLight` and lighting them.
fn light(_p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, sound: &'static str) -> bool {
    let lightable = (logic::is_instance(s, C::CampfireBlock) || logic::is_instance(s, C::AbstractCandleBlock))
        && state::has(s, "lit")
        && !state::get_bool(s, "lit")
        && !state::get_bool(s, "waterlogged");
    if !lightable {
        return false;
    }
    let r = level.random();
    let pitch = if sound == "minecraft:item.firecharge.use" { (r.next_float() - r.next_float()) * 0.2 + 1.0 } else { r.next_float() * 0.4 + 0.8 };
    level.effect(Effect::ActorSound { pos, sound, volume: 1.0, pitch });
    kiln_blocks::set_block(level, pos, state::set_bool(s, "lit", true), flags::ALL_IMMEDIATE);
    true
}

/// `FireChargeItem.useOn`: light a campfire or candle, else fire in front of the face.
fn fire_charge(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, face: Direction, s: u16, off_hand: bool) -> bool {
    let done = if light(p, level, pos, s, "minecraft:item.firecharge.use") {
        true
    } else {
        let at = pos.relative(face);
        let forward = Direction::from_yaw(p.rot[0] as f64);
        if !kiln_blocks::behaviour::portal::fire_can_be_placed_at(level, at, forward) {
            return false;
        }
        let r = level.random();
        let pitch = (r.next_float() - r.next_float()) * 0.2 + 1.0;
        level.effect(Effect::Sound { pos: at, sound: "minecraft:item.firecharge.use", volume: 1.0, pitch });
        let fire = kiln_blocks::behaviour::portal::fire_state(level, at);
        kiln_blocks::set_block_and_update(level, at, fire);
        true
    };
    if done {
        let i = p.hand_index(off_hand);
        kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
        p.inv.times_changed += 1;
    }
    done
}

/// `Mth.nextInt(random, lo, hi)`.
fn next_int_between(r: &mut dyn RandomSource, lo: i32, hi: i32) -> i32 {
    if lo >= hi { lo } else { r.next_int_bounded(hi - lo + 1) + lo }
}

/// `BoneMealItem.useOn`: `growCrop` on a bonemealable block (the bone meal is used up whether
/// or not it took), with the growth particles.
fn bone_meal(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, _face: Direction, off_hand: bool, spawns: &mut Vec<Spawn>) -> bool {
    let s = level.block(pos);
    // Saplings, propagules, azaleas, huge mushrooms, grass: kiln-blocks (with the level's worldgen).
    match kiln_blocks::behaviour::trees::grow_crop(level, pos) {
        Some(true) => {}
        Some(false) => return false,
        None => {
            if !valid_target(level, pos, s) {
                return false;
            }
            if success(level, s) {
                perform(level, pos, s, spawns);
            }
        }
    }
    let i = p.hand_index(off_hand);
    kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
    p.inv.times_changed += 1;
    level.effect(Effect::LevelEvent { id: 1505, pos, data: 15 });
    true
}

fn max_age(s: u16) -> i32 {
    let id = BlockId::of(s);
    let mut max = 0;
    let mut t = id.default_state();
    // The `age` values of the block run from 0 up.
    for a in 0..32 {
        let v = state::set_int(t, "age", a);
        if state::get_int(v, "age") != a {
            break;
        }
        max = a;
        t = v;
    }
    max
}

/// `BonemealableBlock.isValidBonemealTarget`.
fn valid_target(_level: &RegionLevel, _pos: BlockPos, s: u16) -> bool {
    match logic::block_class(s) {
        C::CropBlock | C::BeetrootBlock | C::TorchflowerCropBlock => state::get_int(s, "age") < max_age(s),
        C::StemBlock => state::get_int(s, "age") != 7,
        C::SweetBerryBushBlock => state::get_int(s, "age") < 3,
        C::CocoaBlock => state::get_int(s, "age") < 2,
        C::TallFlowerBlock => true,
        _ => false,
    }
}

/// `BonemealableBlock.isBonemealSuccess`.
fn success(_level: &mut RegionLevel, _s: u16) -> bool {
    true
}

/// `BonemealableBlock.performBonemeal`.
fn perform(level: &mut RegionLevel, pos: BlockPos, s: u16, spawns: &mut Vec<Spawn>) {
    match logic::block_class(s) {
        C::CropBlock | C::BeetrootBlock | C::TorchflowerCropBlock => {
            let increase = match logic::block_class(s) {
                C::TorchflowerCropBlock => 1,
                C::BeetrootBlock => next_int_between(level.random(), 2, 5) / 3,
                _ => next_int_between(level.random(), 2, 5),
            };
            let age = (state::get_int(s, "age") + increase).min(max_age(s));
            kiln_blocks::set_block(level, pos, state::set_int(s, "age", age), flags::CLIENTS);
        }
        C::StemBlock => {
            let age = (state::get_int(s, "age") + next_int_between(level.random(), 2, 5)).min(7);
            kiln_blocks::set_block(level, pos, state::set_int(s, "age", age), flags::CLIENTS);
        }
        C::SweetBerryBushBlock | C::CocoaBlock => {
            let age = state::get_int(s, "age") + 1;
            kiln_blocks::set_block(level, pos, state::set_int(s, "age", age), flags::CLIENTS);
        }
        C::TallFlowerBlock => {
            // `popResource(level, pos, new ItemStack(this))`.
            if let Some(item) = ItemStack::of(BlockId::of(s).name(), 1) {
                let r = level.random();
                let at = [
                    pos.x as f64 + 0.5 + (r.next_double() * 0.5 - 0.25),
                    pos.y as f64 + 0.5 + (r.next_double() * 0.5 - 0.25) - 0.125,
                    pos.z as f64 + 0.5 + (r.next_double() * 0.5 - 0.25),
                ];
                spawns.push(crate::mobs::drop_item(item, at, (pos.x as u64) << 24 ^ pos.z as u64 ^ pos.y as u64));
            }
        }
        _ => {}
    }
}

/// `useItemOn` of blocks that react to the item in hand: cauldrons, composters, pumpkins and
/// shears. `Some(true)`: the item was used; `Some(false)`: the block wants the empty-hand use;
/// `None`: no such block.
pub(crate) fn block_use_item_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, face: Direction, cursor: [f32; 3], off_hand: bool, spawns: &mut Vec<Spawn>) -> Option<bool> {
    if let Some(r) = crate::buckets::use_cauldron(p, level, pos, off_hand, spawns) {
        return Some(r);
    }
    let s = level.block(pos);
    let stack = p.in_hand(off_hand).clone();
    match logic::block_class(s) {
        C::ComposterBlock => compost(p, level, pos, s, off_hand, &stack),
        C::JukeboxBlock => crate::jukebox::use_item_on(p, level, pos, s, off_hand),
        C::CampfireBlock => crate::campfire::use_item_on(p, level, pos, s, off_hand),
        C::BeehiveBlock => crate::beehive::use_item_on(p, level, pos, s, off_hand, spawns),
        C::DecoratedPotBlock => crate::decorated_pot::use_item_on(p, level, pos, s, off_hand),
        C::CakeBlock => cake_candle(p, level, pos, s, off_hand, &stack),
        C::FlowerPotBlock => pot_plant(p, level, pos, s, off_hand, &stack),
        C::ChiseledBookShelfBlock => crate::bookshelf::use_item_on(p, level, pos, s, face, cursor, off_hand, &stack),
        C::PumpkinBlock if !stack.is_empty() && stack.item_name() == "minecraft:shears" => {
            carve(p, level, pos, face, off_hand, spawns);
            Some(true)
        }
        _ => None,
    }
}

/// `ComposterBlock.useWithoutItem`: a full composter gives its bone meal.
pub(crate) fn block_use_without_item(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, face: Direction, cursor: [f32; 3], spawns: &mut Vec<Spawn>) -> bool {
    if crate::jukebox::use_without_item(level, pos, spawns) {
        return true;
    }
    let s = level.block(pos);
    if matches!(logic::block_class(s), C::CakeBlock | C::CandleCakeBlock) {
        return eat_cake(p, level, pos, s);
    }
    if logic::block_class(s) == C::FlowerPotBlock {
        return pot_take(p, level, pos, s, spawns);
    }
    // `DaylightDetectorBlock.useWithoutItem`: a player who may build turns it over.
    if logic::block_class(s) == C::DaylightDetectorBlock && p.game_mode <= 1 {
        kiln_blocks::behaviour::daylight::toggle(level, s, pos);
        return true;
    }
    // `BellBlock.useWithoutItem`: a hit on the body rings it.
    if logic::block_class(s) == C::BellBlock {
        let (proper, rang) = kiln_blocks::behaviour::bell::on_hit(level, pos, face, cursor[1] as f64, true);
        if rang {
            p.award_stat(*crate::player_stats::stat::BELL_RING, 1);
        }
        return proper;
    }
    if logic::block_class(s) == C::ChiseledBookShelfBlock {
        return crate::bookshelf::use_without_item(p, level, pos, s, face, cursor, spawns);
    }
    if logic::block_class(s) == C::DecoratedPotBlock {
        return crate::decorated_pot::use_without_item(level, pos, s);
    }
    if logic::block_class(s) != C::ComposterBlock || state::get_int(s, "level") != 8 {
        return false;
    }
    let r = level.random();
    // `Vec3.atLowerCornerWithOffset(pos, 0.5, 1.01, 0.5).offsetRandomXZ(random, 0.7)`.
    let dx = (r.next_float() - r.next_float()) * 0.7;
    let dz = (r.next_float() - r.next_float()) * 0.7;
    let at = [pos.x as f64 + 0.5 + dx as f64, pos.y as f64 + 1.01, pos.z as f64 + 0.5 + dz as f64];
    if let Some(meal) = ItemStack::of("minecraft:bone_meal", 1) {
        let mut spawn = crate::mobs::drop_item(meal, at, (pos.x as u64) << 32 ^ pos.z as u64);
        spawn.pos = at;
        if let crate::entities::Body::Item { pickup_delay, .. } = &mut spawn.body {
            *pickup_delay = 10;
        }
        spawns.push(spawn);
    }
    kiln_blocks::set_block_and_update(level, pos, state::set_int(s, "level", 0));
    level.effect(Effect::Sound { pos, sound: "minecraft:block.composter.empty", volume: 1.0, pitch: 1.0 });
    true
}

/// The potted block for a plant item (`FlowerPotBlock.POTTED_BY_CONTENT` of its block), if any.
fn potted_for(item: &str) -> Option<u16> {
    let n = item.strip_prefix("minecraft:")?;
    let potted = match n {
        "azalea" => "minecraft:potted_azalea_bush".to_owned(),
        "flowering_azalea" => "minecraft:potted_flowering_azalea_bush".to_owned(),
        _ => format!("minecraft:potted_{n}"),
    };
    kiln_data::blocks_types::block_by_name(&potted).map(|b| b.default)
}

/// The plant item a potted block holds (`new ItemStack(potted)`), none for an empty pot.
fn pot_content(s: u16) -> Option<ItemStack> {
    let name = kiln_blocks::BlockId::of(s).name();
    let n = name.strip_prefix("minecraft:potted_")?;
    let item = match n {
        "azalea_bush" => "azalea",
        "flowering_azalea_bush" => "flowering_azalea",
        other => other,
    };
    ItemStack::of(&format!("minecraft:{item}"), 1)
}

/// `FlowerPotBlock.useItemOn`: a plant item goes into an empty pot (one of it); a full pot takes
/// no second plant (the click is consumed); anything else is for the empty hand's use, which takes
/// the plant out.
fn pot_plant(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, off_hand: bool, stack: &ItemStack) -> Option<bool> {
    let Some(potted) = potted_for(stack.item_name()) else { return Some(false) };
    if pot_content(s).is_some() {
        return Some(true);
    }
    kiln_blocks::set_block_and_update(level, pos, potted);
    level.effect(Effect::GameEvent { pos, event: "minecraft:block_change" });
    p.award_stat(*crate::player_stats::stat::POT_FLOWER, 1);
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
        p.inv.times_changed += 1;
    }
    Some(true)
}

/// `FlowerPotBlock.useWithoutItem`: the plant comes out into the player's hands (or at their feet).
fn pot_take(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, spawns: &mut Vec<Spawn>) -> bool {
    let Some(mut plant) = pot_content(s) else { return true };
    p.add_to_inventory(&mut plant);
    if !plant.is_empty() {
        spawns.push(p.throw(plant));
    }
    if let Some(empty) = kiln_data::blocks_types::block_by_name("minecraft:flower_pot") {
        kiln_blocks::set_block_and_update(level, pos, empty.default);
    }
    level.effect(Effect::GameEvent { pos, event: "minecraft:block_change" });
    true
}

/// `CakeBlock.useItemOn`: a candle on a whole cake makes it a candle cake. Anything else is for
/// the empty hand's use (`TRY_WITH_EMPTY_HAND`).
fn cake_candle(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, off_hand: bool, stack: &ItemStack) -> Option<bool> {
    let name = stack.item_name();
    if state::get_int(s, "bites") != 0 || !name.ends_with("candle") {
        return None;
    }
    let cake = format!("{name}_cake");
    let target = kiln_data::blocks_types::block_by_name(&cake)?.default;
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
        p.inv.times_changed += 1;
    }
    level.effect(Effect::Sound { pos, sound: "minecraft:block.cake.add_candle", volume: 1.0, pitch: 1.0 });
    kiln_blocks::set_block_and_update(level, pos, target);
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: target });
    p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, stack.item()), 1);
    Some(true)
}

/// `CakeBlock.eat`: a slice (2 food, 0.1 saturation) for a player that can eat (or cannot be
/// hurt); the last slice takes the cake. A cake with a candle is eaten as a plain cake would be
/// and gives its candle back (`CandleCakeBlock.useWithoutItem`).
fn eat_cake(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16) -> bool {
    // `Player.canEat(false)`: `abilities.invulnerable || foodData.needsFood()`.
    if !(p.game_mode == 1 || p.food < 20) {
        return false;
    }
    p.award_stat(*crate::player_stats::stat::EAT_CAKE_SLICE, 1);
    // `FoodData.eat(2, 0.1F)`: the saturation is `nutrition * modifier * 2`.
    p.eat(2, 2.0f32 * 0.1f32 * 2.0f32);
    let candle = logic::block_class(s) == C::CandleCakeBlock;
    let bites = if candle { 0 } else { state::get_int(s, "bites") };
    level.effect(Effect::GameEvent { pos, event: "minecraft:eat" });
    if bites < 6 {
        let cake = kiln_data::blocks_types::block_by_name("minecraft:cake").map_or(s, |b| b.default);
        kiln_blocks::set_block(level, pos, state::set_int(cake, "bites", bites + 1), kiln_blocks::flags::ALL);
    } else {
        kiln_blocks::remove_block(level, pos, false);
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_destroy" });
    }
    if candle {
        // `dropResources(state, level, pos)`: the candle cake's loot is its candle.
        level.effect(Effect::Drop { pos, state: s });
    }
    true
}

/// The composter as the `compostable` context providers see it.
struct ComposterContext {
    state: u16,
    origin: [f64; 3],
}

impl kiln_loot::LootContext for ComposterContext {
    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.origin)
    }
    fn block_state(&self) -> Option<u16> {
        Some(self.state)
    }
}

/// `ComposterBlock.useItemOn` with a compostable item: a chance of a layer per item.
fn compost(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, off_hand: bool, stack: &ItemStack) -> Option<bool> {
    let fill = state::get_int(s, "level");
    let compostable = stack.get(kiln_item::keys::COMPOSTABLE)?.clone();
    if fill >= 8 {
        return Some(false);
    }
    if fill < 7 {
        // `addLayer`: the item's layers from its provider, in the composter's context.
        let ctx = ComposterContext { state: s, origin: [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5] };
        let layers = match &compostable.layers {
            kiln_item::component::ResolvableInt::Constant(v) => *v,
            kiln_item::component::ResolvableInt::Reference(id) => {
                let loot = level.env.loot.clone();
                let rng = level.random();
                loot.and_then(|l| l.context_int(id, &ctx, rng)).unwrap_or(0)
            }
        };
        let new = if layers > 0 { (fill + layers).min(7) } else { fill };
        if new != fill {
            kiln_blocks::set_block_and_update(level, pos, state::set_int(s, "level", new));
            if new == 7 {
                kiln_blocks::schedule_block_tick(level, pos, BlockId::of(s), 20, kiln_blocks::TickPriority::Normal);
            }
        }
        level.effect(Effect::LevelEvent { id: 1500, pos, data: (new != fill) as i32 });
        p.award_stat(Stat::item(player_stats::USED, stack.item()), 1);
        if !p.infinite_materials() {
            let i = p.hand_index(off_hand);
            kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
            p.inv.times_changed += 1;
        }
    }
    Some(true)
}

/// `PumpkinBlock.useItemOn` with shears: a carved pumpkin facing the clicked side (the player
/// for top and bottom), four seeds thrown out of it.
fn carve(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, face: Direction, off_hand: bool, spawns: &mut Vec<Spawn>) {
    let dir = if matches!(face, Direction::Up | Direction::Down) { Direction::from_yaw(p.rot[0] as f64).opposite() } else { face };
    level.effect(Effect::Sound { pos, sound: "minecraft:block.pumpkin.carve", volume: 1.0, pitch: 1.0 });
    kiln_blocks::set_block(level, pos, state::set_dir(d::CARVED_PUMPKIN, "facing", dir), flags::ALL_IMMEDIATE);
    let step = dir.step();
    if let Some(seeds) = ItemStack::of("minecraft:pumpkin_seeds", 4) {
        let at = [pos.x as f64 + 0.5 + step[0] as f64 * 0.65, pos.y as f64 + 0.1, pos.z as f64 + 0.5 + step[2] as f64 * 0.65];
        let r = level.random();
        let vel = [0.05 * step[0] as f64 + r.next_double() * 0.02, 0.05, 0.05 * step[2] as f64 + r.next_double() * 0.02];
        spawns.push(Spawn { kind: &kiln_data::entities::types::ITEM, pos: at, vel, body: crate::entities::Body::Item { stack: seeds, pickup_delay: 10, thrower: None } });
    }
    p.hurt_and_break(hand_slot(off_hand), 1, None);
    let shears = p.in_hand(off_hand).item();
    p.award_stat(Stat::item(player_stats::USED, shears), 1);
}
