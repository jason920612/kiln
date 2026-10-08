//! Beehives and bee nests (`BeehiveBlock`, `BeehiveBlockEntity`): up to three bees live inside;
//! a bee that brought nectar spends 2400 ticks there (600 without), is let out in front of the hive
//! (if that is open, and it is not night or raining enough to keep them in) and leaves a level of
//! honey behind. A full hive (honey level 5) gives a honey bottle or, to shears, three honeycombs
//! and lets its bees out angry unless a campfire's smoke calms them. Fire beside the hive or a
//! broken hive sends them all out at once, angry at whoever is near.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::container::{BeKind, pos_random};
use crate::entities::{Body, Entities, Spawn};
use kiln_blocks::{BlockPos, Direction, Effect, Level, state};
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_entity::level::BeehiveView;
use kiln_inventory::Container as _;
use kiln_item::ItemStack;
use kiln_item::component::{BeeOccupant, Bees, Component, EntityData};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

/// `BeehiveBlockEntity.MAX_OCCUPANTS`.
const MAX_OCCUPANTS: usize = 3;

/// `BeehiveBlockEntity.IGNORED_BEE_TAGS`: what a bee does not take into the hive (and does not
/// come out with).
const IGNORED_BEE_TAGS: [&str; 25] = [
    "Air",
    "drop_chances",
    "equipment",
    "Brain",
    "CanPickUpLoot",
    "DeathTime",
    "fall_distance",
    "FallFlying",
    "Fire",
    "HurtTime",
    "LeftHanded",
    "Motion",
    "NoGravity",
    "OnGround",
    "PortalCooldown",
    "Pos",
    "Rotation",
    "sleeping_pos",
    "CannotEnterHiveTicks",
    "TicksSincePollination",
    "CropsGrownSincePollination",
    "hive_pos",
    "Passengers",
    "leash",
    "UUID",
];

/// One bee in the hive: its saved data (with `id`), how long it has been in and the least time it
/// stays (`BeeData` and `Occupant`).
#[derive(Clone, Debug)]
pub(crate) struct Occupant {
    pub data: Tag,
    pub ticks: i32,
    pub min_ticks: i32,
}

impl Occupant {
    fn has_nectar(&self) -> bool {
        self.data.get("HasNectar").and_then(Tag::as_i64).unwrap_or(0) != 0
    }
}

/// `BeehiveBlockEntity.BeeReleaseStatus`: what let a bee out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Release {
    HoneyDelivered,
    BeeReleased,
    Emergency,
}

/// A hive's occupants and the flower they know (`stored`, `savedFlowerPos`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Hive {
    pub occupants: Vec<Occupant>,
    pub flower_pos: Option<BlockPos>,
}

impl Hive {
    pub fn load(nbt: &Tag) -> Hive {
        let mut occupants = Vec::new();
        if let Some(Tag::List(list)) = nbt.get("bees") {
            for t in list {
                let Some(data) = t.get("entity_data").cloned() else { continue };
                let ticks = t.get("ticks_in_hive").and_then(Tag::as_i64).unwrap_or(0) as i32;
                let min_ticks = t.get("min_ticks_in_hive").and_then(Tag::as_i64).unwrap_or(0) as i32;
                occupants.push(Occupant { data, ticks, min_ticks });
            }
        }
        let flower_pos = match nbt.get("flower_pos") {
            Some(Tag::IntArray(v)) if v.len() == 3 => Some(BlockPos::new(v[0], v[1], v[2])),
            _ => None,
        };
        Hive { occupants, flower_pos }
    }

    pub fn save(&self, out: &mut Vec<(String, Tag)>) {
        let bees = self
            .occupants
            .iter()
            .map(|o| {
                Tag::Compound(vec![
                    ("entity_data".into(), o.data.clone()),
                    ("ticks_in_hive".into(), Tag::Int(o.ticks)),
                    ("min_ticks_in_hive".into(), Tag::Int(o.min_ticks)),
                ])
            })
            .collect();
        out.push(("bees".into(), Tag::List(bees)));
        if let Some(p) = self.flower_pos {
            out.push(("flower_pos".into(), Tag::IntArray(vec![p.x, p.y, p.z])));
        }
    }

    pub fn is_full(&self) -> bool {
        self.occupants.len() == MAX_OCCUPANTS
    }

    /// `collectImplicitComponents`: the `bees` component.
    pub fn components(&self) -> Vec<Component> {
        let bees = self
            .occupants
            .iter()
            .filter_map(|o| {
                let entity_data = EntityData::from_value(&kiln_item::Value::from_nbt(&o.data)).ok()?;
                Some(BeeOccupant { entity_data, ticks_in_hive: o.ticks, min_ticks_in_hive: o.min_ticks })
            })
            .collect();
        vec![Component::Bees(Bees(bees))]
    }

    /// `applyImplicitComponents`: the bees of a placed item.
    pub fn apply(&mut self, bees: &Bees) {
        self.occupants = bees
            .0
            .iter()
            .map(|b| Occupant { data: b.entity_data.to_value().to_nbt(), ticks: b.ticks_in_hive, min_ticks: b.min_ticks_in_hive })
            .collect();
    }
}

fn hive<'a>(level: &'a RegionLevel, pos: BlockPos) -> Option<&'a Hive> {
    level.blocks.containers.get(pos).and_then(|c| c.hive.as_deref())
}

fn hive_mut<'a>(level: &'a mut RegionLevel, pos: BlockPos) -> Option<&'a mut Hive> {
    level.blocks.containers.get_mut(pos).and_then(|c| c.hive.as_deref_mut())
}

fn kb(p: BlockPos) -> kiln_entity::math::BlockPos {
    kiln_entity::math::BlockPos::new(p.x, p.y, p.z)
}

/// `BeehiveBlockEntity.isFireNearby`: a `FireBlock` in the 3x3x3 around the hive.
pub(crate) fn is_fire_nearby(level: &RegionLevel, pos: BlockPos) -> bool {
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                if logic::block_class(level.block(pos.offset(x, y, z))) == C::FireBlock {
                    return true;
                }
            }
        }
    }
    false
}

/// `CampfireBlock.isSmokeyPos`: a lit campfire up to five blocks below, or just below a block
/// that fills the middle of the column (a post or hay bale in the way smoke passes).
pub(crate) fn is_smokey_pos(level: &RegionLevel, pos: BlockPos) -> bool {
    let lit_campfire = |s: u16| logic::block_class(s) == C::CampfireBlock && state::get_bool(s, "lit");
    for i in 1..=5 {
        let below = pos.offset(0, -i, 0);
        let s = level.block(below);
        if lit_campfire(s) {
            return true;
        }
        // `VIRTUAL_FENCE_POST` = box(6, 0, 6, 10, 16, 10) against the block's collision shape.
        let post = kiln_data::block_props::collision(s).iter().any(|b| b[0] < 0.625 && b[3] > 0.375 && b[2] < 0.625 && b[5] > 0.375 && b[1] < 1.0 && b[4] > 0.0);
        if post {
            return lit_campfire(level.block(below.below()));
        }
    }
    false
}

/// The `minecraft:gameplay/bees_stay_in_hive` attribute: the overworld from tick 12542 to 23460
/// of its day.
pub(crate) fn bees_stay_in_hive(env: &crate::blocks::BlockEnv) -> bool {
    env.dim == crate::OVERWORLD_ID && (12542..23460).contains(&env.mobs.day_time.rem_euclid(24000))
}

/// What bees see of the hive at `pos`.
pub(crate) fn view(level: &RegionLevel, pos: BlockPos) -> Option<BeehiveView> {
    let h = hive(level, pos)?;
    Some(BeehiveView { full: h.is_full(), fire_nearby: is_fire_nearby(level, pos) })
}

/// A sound at exact coordinates (`Level.playSound(null, x, y, z, ...)`).
fn sound_at(level: &mut RegionLevel, at: [f64; 3], sound: &str, volume: f32, pitch: f32, salt: usize) {
    if let Some(pkt) = crate::blocks::sound_packet_at(sound, at, volume, pitch, level.env, salt) {
        level.out.packets.push((at, 16.0 * volume.max(1.0) as f64, pkt));
    }
}

/// `BeehiveBlockEntity.addOccupant(bee)` for a bee saved as `data`: the bee goes into the hive
/// (its caller discards it). `bee_flower` is its `savedFlowerPos`.
pub(crate) fn add_occupant(level: &mut RegionLevel, pos: BlockPos, data: Tag, bee_flower: Option<BlockPos>) -> bool {
    let Some(h) = hive(level, pos) else { return false };
    if h.occupants.len() >= MAX_OCCUPANTS {
        return false;
    }
    let had_flower = h.flower_pos.is_some();
    let data = match data {
        Tag::Compound(f) => Tag::Compound(f.into_iter().filter(|(k, _)| !IGNORED_BEE_TAGS.contains(&k.as_str())).collect()),
        other => other,
    };
    let nectar = data.get("HasNectar").and_then(Tag::as_i64).unwrap_or(0) != 0;
    if let Some(h) = hive_mut(level, pos) {
        h.occupants.push(Occupant { data, ticks: 0, min_ticks: if nectar { 2400 } else { 600 } });
    }
    // `bee.hasSavedFlowerPos() && (!hasSavedFlowerPos() || random.nextBoolean())`.
    if let Some(f) = bee_flower
        && (!had_flower || level.random().next_bool())
        && let Some(h) = hive_mut(level, pos)
    {
        h.flower_pos = Some(f);
    }
    sound_at(level, [pos.x as f64, pos.y as f64, pos.z as f64], "minecraft:block.beehive.enter", 1.0, 1.0, 0);
    let s = level.block(pos);
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: s });
    if let Some(c) = level.blocks.containers.get_mut(pos) {
        c.mark_changed();
    }
    true
}

/// `BeehiveBlockEntity.releaseOccupant`: whether the bee came out. `state_` is the hive's state
/// (the destroyed hive's, when it was broken); `player` is who the bees turn on, if anyone.
#[allow(clippy::too_many_arguments)]
fn release_occupant(
    level: &mut RegionLevel,
    pos: BlockPos,
    state_: u16,
    o: &Occupant,
    status: Release,
    saved_flower: Option<BlockPos>,
    player: Option<(i32, [f64; 3])>,
    salt: usize,
) -> bool {
    if bees_stay_in_hive(level.env) && status != Release::Emergency {
        return false;
    }
    let facing = state::get_dir(state_, "facing").unwrap_or(Direction::North);
    let front = pos.relative(facing);
    let blocked = !kiln_data::block_props::collision(level.block(front)).is_empty();
    if blocked && status != Release::Emergency {
        return false;
    }
    // `Occupant.createEntity`.
    let Tag::Compound(fields) = &o.data else { return false };
    let tag = Tag::Compound(fields.iter().filter(|(k, _)| !IGNORED_BEE_TAGS.contains(&k.as_str())).cloned().collect());
    let seed = pos_random(level, pos, 0x6265_6500 + salt as u64).next_long();
    let Ok(mut e) = kiln_entity::persist::load(&tag, 0, seed) else { return false };
    if e.type_name != "minecraft:bee" {
        return false;
    }
    kiln_entity::mob::kinds::bee::release_setup(&mut e, kb(pos), o.ticks);
    if let Some(f) = saved_flower
        && kiln_entity::mob::kinds::bee::saved_flower_pos(&e).is_none()
        && level.random().next_float() < 0.9
    {
        kiln_entity::mob::kinds::bee::set_saved_flower_pos(&mut e, kb(f));
    }
    if status == Release::HoneyDelivered {
        kiln_entity::mob::kinds::bee::drop_off_nectar(&mut e);
        if logic::block_class(state_) == C::BeehiveBlock && state::has(state_, "honey_level") {
            let honey = state::get_int(state_, "honey_level");
            if honey < 5 {
                let mut increment = if level.random().next_int_bounded(100) == 0 { 2 } else { 1 };
                if honey + increment > 5 {
                    increment -= 1;
                }
                // `setBlockAndUpdate` on the hive's current state.
                let now = level.block(pos);
                kiln_blocks::set_block_and_update(level, pos, state::set_int(now, "honey_level", honey + increment));
            }
        }
    }
    let width = e.width as f64;
    let xz = if blocked { 0.0 } else { 0.55 + (width as f32 / 2.0) as f64 };
    let step = facing.step();
    let x = pos.x as f64 + 0.5 + xz * step[0] as f64;
    let y = pos.y as f64 + 0.5 - (e.height / 2.0) as f64;
    let z = pos.z as f64 + 0.5 + xz * step[2] as f64;
    e.set_pos(kiln_entity::math::Vec3::new(x, y, z));
    e.set_old_pos_and_rot();
    // `emptyAllLivingFromHive`: bees within 4 blocks of the player turn on them, unless smoke
    // calms them.
    if let Some((pid, p)) = player {
        let d = (p[0] - x).powi(2) + (p[1] - y).powi(2) + (p[2] - z).powi(2);
        if d <= 16.0 {
            if !is_smokey_pos(level, pos) {
                kiln_entity::mob::kinds::bee::set_target(&mut e, pid);
            } else {
                kiln_entity::mob::kinds::bee::set_stay_out_of_hive(&mut e, 400);
            }
        }
    }
    level.effect(Effect::Sound { pos, sound: "minecraft:block.beehive.exit", volume: 1.0, pitch: 1.0 });
    let now = level.block(pos);
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: now });
    if let Some(kind) = kiln_data::entities::by_name("minecraft:bee") {
        level.out.spawns.push(Spawn { kind, pos: [x, y, z], vel: [0.0; 3], body: Body::Ready(Box::new(e)) });
    }
    true
}

/// `BeehiveBlockEntity.emptyAllLivingFromHive(player, state, status)`.
pub(crate) fn empty_all(level: &mut RegionLevel, pos: BlockPos, state_: u16, player: Option<(i32, [f64; 3])>, status: Release) {
    let Some(h) = hive(level, pos) else { return };
    let (saved_flower, occupants) = (h.flower_pos, h.occupants.clone());
    let mut kept = Vec::new();
    let mut changed = false;
    for (i, o) in occupants.into_iter().enumerate() {
        if release_occupant(level, pos, state_, &o, status, saved_flower, player, i) {
            changed = true;
        } else {
            kept.push(o);
        }
    }
    if let Some(h) = hive_mut(level, pos) {
        h.occupants = kept;
    }
    if changed && let Some(c) = level.blocks.containers.get_mut(pos) {
        c.mark_changed();
    }
}

/// `BeehiveBlockEntity.serverTick`.
pub(crate) fn tick(level: &mut RegionLevel, pos: BlockPos) {
    let Some(h) = hive(level, pos) else { return };
    let saved_flower = h.flower_pos;
    let state_ = level.block(pos);
    let mut changed = false;
    let mut i = 0;
    while let Some(o) = hive(level, pos).and_then(|h| h.occupants.get(i)).cloned() {
        // `BeeData.tick`: `ticksInHive++ > minTicksInHive`.
        let ready = o.ticks > o.min_ticks;
        let mut o = o;
        o.ticks += 1;
        if let Some(h) = hive_mut(level, pos) {
            h.occupants[i].ticks = o.ticks;
        }
        if ready {
            let status = if o.has_nectar() { Release::HoneyDelivered } else { Release::BeeReleased };
            if release_occupant(level, pos, state_, &o, status, saved_flower, None, i) {
                if let Some(h) = hive_mut(level, pos) {
                    h.occupants.remove(i);
                }
                changed = true;
                continue;
            }
        }
        i += 1;
    }
    if changed && let Some(c) = level.blocks.containers.get_mut(pos) {
        c.mark_changed();
    }
    if hive(level, pos).is_some_and(|h| !h.occupants.is_empty()) && level.random().next_double() < 0.005 {
        sound_at(level, [pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5], "minecraft:block.beehive.work", 1.0, 1.0, 1);
    }
}

/// `BeehiveBlock.updateShape`: a fire beside the hive sends the bees out.
pub(crate) fn neighbour_fire(level: &mut RegionLevel, pos: BlockPos, s: u16) {
    if hive(level, pos).is_some() {
        empty_all(level, pos, s, None, Release::Emergency);
    }
}

/// `BeehiveBlock.playerDestroy` for a hive about to go: its bees come out (and the ones nearby turn
/// on the player) unless the tool prevents it (silk touch).
pub(crate) fn player_destroy(level: &mut RegionLevel, pos: BlockPos, s: u16, p: &Player) {
    if hive(level, pos).is_none() {
        return;
    }
    let silk = p
        .inv
        .selected_item()
        .get(kiln_item::keys::ENCHANTMENTS)
        .is_some_and(|e| kiln_item::registry::ENCHANTMENT.id("minecraft:silk_touch").is_some_and(|id| e.level(id) > 0));
    if silk {
        return;
    }
    empty_all(level, pos, s, Some((p.entity_id, p.pos)), Release::Emergency);
    level.blocks.bee_anger.push(pos);
}

/// `BeehiveBlock.playerWillDestroy`: a creative player breaking a hive with bees or honey still gets
/// it as an item (with its bees and honey level).
pub(crate) fn player_will_destroy(level: &mut RegionLevel, pos: BlockPos, s: u16, creative: bool) {
    if logic::block_class(s) != C::BeehiveBlock || !creative || !level.env.drops {
        return;
    }
    let Some(h) = hive(level, pos) else { return };
    let honey = state::get_int(s, "honey_level");
    if h.occupants.is_empty() && honey <= 0 {
        return;
    }
    let Some(mut item) = ItemStack::of(kiln_blocks::BlockId::of(s).name(), 1) else { return };
    for c in h.components() {
        item.set(c);
    }
    item.set(Component::BlockState(kiln_item::component::BlockItemStateProperties(vec![("honey_level".to_owned(), honey.to_string())])));
    level.out.spawns.push(Spawn {
        kind: &kiln_data::entities::types::ITEM,
        pos: [pos.x as f64, pos.y as f64, pos.z as f64],
        vel: [0.0; 3],
        body: Body::Item { stack: item, pickup_delay: 10, thrower: None },
    });
}

/// `BeehiveBlock.useItemOn`: shears or a glass bottle on a full hive. `None`: not for the hive.
pub(crate) fn use_item_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, off_hand: bool, spawns: &mut Vec<Spawn>) -> Option<bool> {
    if logic::block_class(s) != C::BeehiveBlock || state::get_int(s, "honey_level") < 5 {
        return None;
    }
    let stack = p.in_hand(off_hand).clone();
    let name = stack.item_name();
    let item = stack.item();
    let used = if name == "minecraft:shears" {
        // `dropHoneycomb`: the `harvest/beehive` table (three honeycombs) popped out of the block.
        if let Some(comb) = ItemStack::of("minecraft:honeycomb", 3) {
            let r = level.random();
            let at = [
                pos.x as f64 + 0.5 + (r.next_double() * 0.5 - 0.25),
                pos.y as f64 + 0.5 + (r.next_double() * 0.5 - 0.25) - 0.125,
                pos.z as f64 + 0.5 + (r.next_double() * 0.5 - 0.25),
            ];
            let mut spawn = crate::mobs::drop_item(comb, at, (pos.x as u64) << 24 ^ pos.z as u64 ^ pos.y as u64);
            spawn.pos = at;
            if let Body::Item { pickup_delay, .. } = &mut spawn.body {
                *pickup_delay = 10;
            }
            spawns.push(spawn);
        }
        sound_at(level, p.pos, "minecraft:block.beehive.shear", 1.0, 1.0, 2);
        p.hurt_and_break(crate::tools::hand_slot(off_hand), 1, None);
        level.effect(Effect::GameEvent { pos, event: "minecraft:shear" });
        true
    } else if name == "minecraft:glass_bottle" {
        let i = p.hand_index(off_hand);
        kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
        p.inv.times_changed += 1;
        // The player's own client plays the filling sound.
        if let Some(mut honey) = ItemStack::of("minecraft:honey_bottle", 1) {
            if p.inv.item(i).is_empty() {
                p.set_in_hand(off_hand, honey);
            } else {
                p.add_to_inventory(&mut honey);
                if !honey.is_empty() {
                    spawns.push(p.throw(honey));
                }
            }
        }
        level.effect(Effect::GameEvent { pos, event: "minecraft:fluid_pickup" });
        true
    } else {
        false
    };
    if !used {
        return None;
    }
    p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, item), 1);
    if !is_smokey_pos(level, pos) {
        if hive(level, pos).is_some_and(|h| !h.occupants.is_empty()) {
            level.blocks.bee_anger.push(pos);
        }
        // `releaseBeesAndResetHoneyLevel`.
        reset_honey(level, pos, s);
        empty_all(level, pos, s, Some((p.entity_id, p.pos)), Release::Emergency);
    } else {
        reset_honey(level, pos, s);
    }
    Some(true)
}

/// `BeehiveBlock.resetHoneyLevel`.
fn reset_honey(level: &mut RegionLevel, pos: BlockPos, s: u16) {
    kiln_blocks::set_block_and_update(level, pos, state::set_int(s, "honey_level", 0));
}

/// `BeehiveBlock.angerNearbyBees` for the requests block work made: every bee within 8 x 6 x 8 of
/// the hive that has no target takes a random player of that box as one.
pub(crate) fn anger_requests(level: &mut RegionLevel, entities: &mut Entities, players: &[&mut Player]) {
    if level.blocks.bee_anger.is_empty() {
        return;
    }
    for pos in std::mem::take(&mut level.blocks.bee_anger) {
        let (lo, hi) = ([pos.x as f64 - 8.0, pos.y as f64 - 6.0, pos.z as f64 - 8.0], [pos.x as f64 + 9.0, pos.y as f64 + 7.0, pos.z as f64 + 9.0]);
        let hits = |min: [f64; 3], max: [f64; 3]| (0..3).all(|i| min[i] < hi[i] && max[i] > lo[i]);
        let bees: Vec<usize> = entities
            .list
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                !e.removed && e.kind.name == "minecraft:bee" && {
                    let (min, max, _) = e.body();
                    hits(min, max)
                }
            })
            .map(|(i, _)| i)
            .collect();
        if bees.is_empty() {
            continue;
        }
        let near: Vec<i32> = players
            .iter()
            .filter(|p| {
                let h = p.dimensions().1 as f64;
                hits([p.pos[0] - 0.3, p.pos[1], p.pos[2] - 0.3], [p.pos[0] + 0.3, p.pos[1] + h, p.pos[2] + 0.3])
            })
            .map(|p| p.entity_id)
            .collect();
        if near.is_empty() {
            continue;
        }
        for i in bees {
            let Some(phys) = entities.list[i].phys.as_deref_mut() else { continue };
            if kiln_entity::mob::kinds::bee::has_target(phys) {
                continue;
            }
            let pick = near[level.random().next_int_bounded(near.len() as i32) as usize];
            kiln_entity::mob::kinds::bee::set_target(phys, pick);
        }
    }
}

/// Whether `pos` holds a hive block entity.
pub(crate) fn exists(level: &RegionLevel, pos: BlockPos) -> bool {
    level.blocks.containers.get(pos).is_some_and(|c| c.kind == BeKind::Beehive)
}
