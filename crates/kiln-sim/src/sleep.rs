//! Sleeping and respawn blocks: beds (`AbstractBedBlock.useWithoutItem`,
//! `ServerPlayer.startSleepInBed`), the sleeping players of each level and the night skip
//! (`SleepStatus`, `ServerLevel.tick`), waking up, respawn anchors (charging with glowstone,
//! setting the spawn, exploding where they do not work) and respawning at either.
//!
//! The insomnia counter (`minecraft:time_since_rest`) is kept on the player here and read by
//! the phantom spawner; it is saved under that key of the vanilla statistics file.

use crate::blocks::{EntityBox, RegionLevel};
use crate::{DimId, NETHER_ID, OVERWORLD_ID, Player, Sim};
use kiln_blocks::{BlockPos, Direction, Effect, Level, flags};
use kiln_data::block_logic::{self as logic, BlockClass};
use kiln_data::blocks::default_state as d;
use kiln_proto::packets;

/// A player's sleep: where, for how long, and the insomnia statistic.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sleep {
    /// `LivingEntity.getSleepingPos`: the bed's head.
    pub pos: Option<[i32; 3]>,
    /// `Player.sleepCounter`: up to 100 while asleep, then 101..110 fading out after waking.
    pub counter: i32,
    /// `minecraft:time_since_rest`.
    pub time_since_rest: i32,
    /// The level's sleeping list needs updating (`updateSleepingPlayerList`).
    pub list_dirty: bool,
    /// Pose and sleeping position changed for viewers and the player.
    pub meta_dirty: bool,
}

/// `BedRule.Rule`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum When {
    Always,
    WhenDark,
    Never,
}

/// The `minecraft:gameplay/bed_rule` of a level.
#[derive(Clone, Copy, Debug)]
struct BedRule {
    can_sleep: When,
    can_set_spawn: When,
    destroy_on_use: bool,
    error: Option<&'static str>,
}

fn bed_rule(dim: DimId) -> BedRule {
    if dim == OVERWORLD_ID {
        BedRule { can_sleep: When::WhenDark, can_set_spawn: When::Always, destroy_on_use: false, error: Some("block.minecraft.bed.no_sleep") }
    } else {
        BedRule { can_sleep: When::Never, can_set_spawn: When::Never, destroy_on_use: true, error: None }
    }
}

/// `BedRule.Rule.test`: `WHEN_DARK` is `Level.isDarkOutside` (sky darkened by 4 or more in a
/// level without fixed time).
fn test(when: When, dim: DimId, sky_darken: i32) -> bool {
    match when {
        When::Always => true,
        When::Never => false,
        When::WhenDark => dim == OVERWORLD_ID && sky_darken >= 4,
    }
}

/// `minecraft:gameplay/respawn_anchor_works` (the nether only).
fn respawn_anchor_works(dim: DimId) -> bool {
    dim == NETHER_ID
}

fn is_bed(s: u16) -> bool {
    logic::is_instance(s, BlockClass::AbstractBedBlock)
}

fn is_anchor(s: u16) -> bool {
    logic::block_class(s) == BlockClass::RespawnAnchorBlock
}

fn facing(s: u16) -> Direction {
    kiln_blocks::state::get_dir(s, "facing").unwrap_or(Direction::North)
}

fn overlay(p: &mut Player, key: &str) {
    p.send(packets::system_chat(kiln_command::tr!(key).to_nbt(), true));
}

fn center(pos: BlockPos) -> [f64; 3] {
    [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5]
}

/// Whether a click on `pos` goes to a bed or respawn anchor (handled by [`use_block`]).
pub(crate) fn handles(state: u16) -> bool {
    is_bed(state) || is_anchor(state)
}

/// What a click on a bed or respawn anchor did besides the blocks: an explosion to set off
/// at a position (in the region, with its entities).
pub(crate) enum Outcome {
    Nothing,
    Explode([f64; 3]),
}

/// The block part of `useItemOn` / `useWithoutItem` for beds and respawn anchors, for the
/// main hand. Returns whether the click was consumed, and what else must happen.
pub(crate) fn use_block(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, held: &str, offhand: &str) -> (bool, Outcome) {
    let s = level.block(pos);
    if is_anchor(s) {
        return use_anchor(p, level, pos, s, held, offhand);
    }
    use_bed(p, level, pos, s)
}

fn use_anchor(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, held: &str, offhand: &str) -> (bool, Outcome) {
    let charge = kiln_blocks::state::get_int(s, "charges");
    // `useItemOn`: glowstone charges the anchor.
    if held == "minecraft:glowstone" && charge < 4 {
        let charged = kiln_blocks::state::set_int(s, "charges", charge + 1);
        kiln_blocks::set_block(level, pos, charged, flags::ALL);
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_change" });
        level.effect(Effect::Sound { pos, sound: "minecraft:block.respawn_anchor.charge", volume: 1.0, pitch: 1.0 });
        if p.game_mode != 1 {
            let slot = kiln_inventory::inventory::equipment_index(kiln_item::component::EquipmentSlot::MainHand, p.inv.selected);
            kiln_inventory::Container::item_mut(&mut p.inv, slot).shrink(1);
        }
        return (true, Outcome::Nothing);
    }
    if offhand == "minecraft:glowstone" && charge < 4 {
        // `PASS`: the off hand charges it.
        return (false, Outcome::Nothing);
    }
    // `useWithoutItem`.
    if charge == 0 {
        return (false, Outcome::Nothing);
    }
    if !respawn_anchor_works(level.env.dim) {
        kiln_blocks::remove_block(level, pos, false);
        return (true, Outcome::Explode(center(pos)));
    }
    let dim = level.env.dim;
    if !(p.respawn == Some([pos.x, pos.y, pos.z]) && p.respawn_dim == dim) {
        set_respawn(p, dim, [pos.x, pos.y, pos.z], 0.0, false);
        level.effect(Effect::Sound { pos, sound: "minecraft:block.respawn_anchor.set_spawn", volume: 1.0, pitch: 1.0 });
    }
    (true, Outcome::Nothing)
}

/// `ServerPlayer.setRespawnPosition(config, true)`: tells the player when the point moved.
fn set_respawn(p: &mut Player, dim: DimId, pos: [i32; 3], angle: f32, forced: bool) {
    let same = p.respawn == Some(pos) && p.respawn_dim == dim;
    if !same {
        p.send(packets::system_chat(kiln_command::tr!("block.minecraft.set_spawn").to_nbt(), false));
    }
    p.respawn = Some(pos);
    p.respawn_dim = dim;
    p.respawn_angle = angle;
    p.respawn_forced = forced;
}

fn use_bed(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16) -> (bool, Outcome) {
    // The head half is the bed; a foot without its head does nothing.
    let (pos, s) = if kiln_blocks::state::get(s, "part") != Some("head") {
        let head = pos.relative(facing(s));
        let hs = level.block(head);
        if !kiln_blocks::state::same_block(hs, s) {
            return (true, Outcome::Nothing);
        }
        (head, hs)
    } else {
        (pos, s)
    };
    let dim = level.env.dim;
    let rule = bed_rule(dim);
    if rule.destroy_on_use {
        // `BedBlock.destroyOnUse`: both halves go and the bed explodes.
        kiln_blocks::remove_block(level, pos, false);
        let foot = pos.relative(facing(s).opposite());
        if kiln_blocks::state::same_block(level.block(foot), s) {
            kiln_blocks::remove_block(level, foot, false);
        }
        return (true, Outcome::Explode(center(pos)));
    }
    if kiln_blocks::state::get_bool(s, "occupied") {
        overlay(p, "block.minecraft.bed.occupied");
        return (true, Outcome::Nothing);
    }
    if let Err(problem) = start_sleep_in_bed(p, level, pos, s, rule) {
        if let Some(key) = problem {
            overlay(p, key);
        }
    }
    (true, Outcome::Nothing)
}

/// `ServerPlayer.startSleepInBed`: the problem's message key on failure (`None` for
/// `OTHER_PROBLEM`).
fn start_sleep_in_bed(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, rule: BedRule) -> Result<(), Option<&'static str>> {
    let dir = facing(s);
    if p.sleep.pos.is_some() || p.dead {
        return Err(None);
    }
    let dim = level.env.dim;
    let sky_darken = level.env.mobs.sky_darken;
    let can_sleep = test(rule.can_sleep, dim, sky_darken);
    let can_set_spawn = test(rule.can_set_spawn, dim, sky_darken);
    let reachable = |b: BlockPos| {
        let c = [b.x as f64 + 0.5, b.y as f64, b.z as f64 + 0.5];
        (p.pos[0] - c[0]).abs() <= 3.0 && (p.pos[1] - c[1]).abs() <= 2.0 && (p.pos[2] - c[2]).abs() <= 3.0
    };
    if !reachable(pos) && !reachable(pos.relative(dir.opposite())) {
        return Err(Some("block.minecraft.bed.too_far_away"));
    }
    let free = |b: BlockPos| !kiln_entity::physics::is_suffocating(level.block(b));
    if !free(pos.above()) || !free(pos.relative(dir.opposite()).above()) {
        return Err(Some("block.minecraft.bed.obstructed"));
    }
    if can_set_spawn {
        set_respawn(p, dim, [pos.x, pos.y, pos.z], p.rot[0], false);
    }
    if !can_sleep {
        return Err(rule.error.or(Some("block.minecraft.bed.no_sleep")));
    }
    if p.game_mode != 1 {
        let c = [pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5];
        let (min, max) = ([c[0] - 8.0, c[1] - 5.0, c[2] - 8.0], [c[0] + 8.0, c[1] + 5.0, c[2] + 8.0]);
        if level.bodies.iter().any(|b: &EntityBox| b.prevents_rest && (0..3).all(|i| b.min[i] < max[i] && b.max[i] > min[i])) {
            return Err(Some("block.minecraft.bed.not_safe"));
        }
    }
    start_sleeping(p, level, pos, s);
    Ok(())
}

/// `LivingEntity.startSleeping` + `ServerPlayer.startSleeping`: the player lies on the bed
/// (0.6875 above its block), the bed is occupied, the insomnia counter resets and the client
/// is moved there.
fn start_sleeping(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16) {
    const SLEEP_HEIGHT: f64 = 0.5625;
    p.vehicle = None;
    p.pos = [pos.x as f64 + 0.5, pos.y as f64 + SLEEP_HEIGHT + 0.125, pos.z as f64 + 0.5];
    kiln_blocks::set_block(level, pos, kiln_blocks::state::set_bool(s, "occupied", true), flags::ALL);
    p.sleep.pos = Some([pos.x, pos.y, pos.z]);
    p.sleep.counter = 0;
    p.sleep.time_since_rest = 0;
    p.sleep.list_dirty = true;
    p.sleep.meta_dirty = true;
    p.vel = [0.0; 3];
    p.meta_dirty = true;
    let (pos, rot, now) = (p.pos, p.rot, level.env.game_time);
    p.teleport(pos, rot, now);
}

/// `ServerPlayer.stopSleepInBed(wakeImmediately, updateLevelList)`: the wake-up animation for
/// everyone watching, the bed freed and the player standing up beside it.
pub(crate) fn stop_sleep_in_bed(p: &mut Player, level: &mut RegionLevel, wake_immediately: bool, update_list: bool) {
    let Some(at) = p.sleep.pos else { return };
    // `ClientboundAnimatePacket.WAKE_UP` to the tracking players and the player.
    p.woke_up = true;
    let bed = BlockPos::new(at[0], at[1], at[2]);
    let s = level.block(bed);
    if level.is_loaded(bed) && is_bed(s) {
        let dir = facing(s);
        kiln_blocks::set_block(level, bed, kiln_blocks::state::set_bool(s, "occupied", false), flags::ALL);
        let stand = bed_stand_up(level, bed, dir, p.rot[0]).unwrap_or([at[0] as f64 + 0.5, at[1] as f64 + 1.0 + 0.1, at[2] as f64 + 0.5]);
        let to_bed = [bed.x as f64 + 0.5 - stand[0], bed.y as f64 - stand[1], bed.z as f64 + 0.5 - stand[2]];
        let len = (to_bed[0] * to_bed[0] + to_bed[1] * to_bed[1] + to_bed[2] * to_bed[2]).sqrt();
        let (nx, nz) = if len < 1.0e-4 { (0.0, 0.0) } else { (to_bed[0] / len, to_bed[2] / len) };
        let yaw = kiln_entity::mob::mth::wrap_degrees((nz.atan2(nx) * 57.295_776_367_187_5 - 90.0) as f32);
        p.pos = stand;
        p.rot = [yaw, 0.0];
    }
    p.sleep.pos = None;
    p.sleep.meta_dirty = true;
    p.meta_dirty = true;
    if update_list {
        p.sleep.list_dirty = true;
    }
    p.sleep.counter = if wake_immediately { 0 } else { 100 };
    let (pos, rot, now) = (p.pos, p.rot, level.env.game_time);
    p.teleport(pos, rot, now);
}

/// `Player.tick`'s sleep part and the insomnia statistic (`ServerPlayer.doTick`): asleep, the
/// counter climbs to 100 and the player wakes when the bed rule stops allowing sleep (day) or
/// the bed is gone; awake, the counter runs out after 10 more ticks.
pub(crate) fn tick_player(p: &mut Player, level: &mut RegionLevel) {
    if let Some(at) = p.sleep.pos {
        p.sleep.counter = (p.sleep.counter + 1).min(100);
        let bed = BlockPos::new(at[0], at[1], at[2]);
        if !is_bed(level.block(bed)) {
            // `LivingEntity.checkBedExists`.
            stop_sleep_in_bed(p, level, true, true);
        } else {
            let dim = level.env.dim;
            if !test(bed_rule(dim).can_sleep, dim, level.env.mobs.sky_darken) {
                stop_sleep_in_bed(p, level, false, true);
            }
        }
    } else if p.sleep.counter > 0 {
        p.sleep.counter += 1;
        if p.sleep.counter >= 110 {
            p.sleep.counter = 0;
        }
    }
    if p.sleep.pos.is_none() {
        p.sleep.time_since_rest = p.sleep.time_since_rest.saturating_add(1);
    }
}

/// `CollisionGetter`-lite: the block collision boxes (in world space) intersecting a box.
fn collides<L: Level + ?Sized>(level: &L, min: [f64; 3], max: [f64; 3]) -> bool {
    let (x0, y0, z0) = (min[0].floor() as i32, min[1].floor() as i32 - 1, min[2].floor() as i32);
    let (x1, y1, z1) = (max[0].floor() as i32, max[1].floor() as i32, max[2].floor() as i32);
    for x in x0..=x1 {
        for y in y0..=y1 {
            for z in z0..=z1 {
                let s = level.block(BlockPos::new(x, y, z));
                for b in kiln_data::block_props::collision(s) {
                    let bmin = [x as f64 + b[0] as f64, y as f64 + b[1] as f64, z as f64 + b[2] as f64];
                    let bmax = [x as f64 + b[3] as f64, y as f64 + b[4] as f64, z as f64 + b[5] as f64];
                    if (0..3).all(|i| bmin[i] < max[i] - 1.0e-7 && bmax[i] > min[i] + 1.0e-7) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// `DismountHelper.nonClimbableShape`'s top: the block's collision top, ignoring climbable
/// blocks; `None` for an empty shape.
fn floor_top(s: u16) -> Option<f64> {
    if kiln_blocks::tags::is(s, "minecraft:climbable") {
        return None;
    }
    kiln_data::block_props::collision(s).iter().map(|b| b[4] as f64).reduce(f64::max)
}

/// `EntityType.isBlockDangerous` for players: fire, lava, magma, campfires and the like.
fn dangerous(s: u16) -> bool {
    kiln_blocks::tags::is(s, "minecraft:fire")
        || kiln_blocks::tags::is(s, "minecraft:campfires")
        || [d::MAGMA_BLOCK, d::LAVA, d::WITHER_ROSE, d::SWEET_BERRY_BUSH, d::CACTUS, d::POWDER_SNOW].iter().any(|&b| kiln_blocks::state::same_block(s, b))
}

/// `DismountHelper.findSafeDismountLocation` for a player at block `pos`.
fn safe_dismount<L: Level + ?Sized>(level: &L, pos: BlockPos, check_dangerous: bool) -> Option<[f64; 3]> {
    if check_dangerous && dangerous(level.block(pos)) {
        return None;
    }
    let floor = match floor_top(level.block(pos)) {
        Some(t) => t,
        None => match floor_top(level.block(pos.below())) {
            Some(t) if t >= 1.0 => t - 1.0,
            _ => f64::NEG_INFINITY,
        },
    };
    if !(floor.is_finite() && floor < 1.0) {
        return None;
    }
    if check_dangerous && floor <= 0.0 && dangerous(level.block(pos.below())) {
        return None;
    }
    let v = [pos.x as f64 + 0.5, pos.y as f64 + floor, pos.z as f64 + 0.5];
    let (min, max) = ([v[0] - 0.3, v[1], v[2] - 0.3], [v[0] + 0.3, v[1] + 1.8, v[2] + 0.3]);
    if collides(level, min, max) {
        return None;
    }
    Some(v)
}

/// `AbstractBedBlock.findStandUpPosition` (bunk beds are treated as plain beds).
pub(crate) fn bed_stand_up<L: Level + ?Sized>(level: &L, head: BlockPos, dir: Direction, yaw: f32) -> Option<[f64; 3]> {
    let cw = dir.clockwise();
    let rot = if facing_angle(cw, yaw) { cw.opposite() } else { cw };
    let (dx, dz) = (dir.step()[0], dir.step()[2]);
    let (rx, rz) = (rot.step()[0], rot.step()[2]);
    let offsets = [
        [rx, rz],
        [rx - dx, rz - dz],
        [rx - dx * 2, rz - dz * 2],
        [-dx * 2, -dz * 2],
        [-rx - dx * 2, -rz - dz * 2],
        [-rx - dx, -rz - dz],
        [-rx, -rz],
        [-rx + dx, -rz + dz],
        [dx, dz],
        [rx + dx, rz + dz],
        [0, 0],
        [-dx, -dz],
    ];
    for check in [true, false] {
        for o in offsets {
            if let Some(v) = safe_dismount(level, BlockPos::new(head.x + o[0], head.y, head.z + o[1]), check) {
                return Some(v);
            }
        }
    }
    None
}

/// `Direction.isFacingAngle`.
fn facing_angle(dir: Direction, yaw: f32) -> bool {
    // `Mth.sin` and `Mth.cos` (the table) of the angle in float radians.
    let r = (yaw * 0.017_453_292_f32) as f64;
    let (x, z) = (-kiln_entity::mob::mth::sin(r), kiln_entity::mob::mth::cos(r));
    let s = dir.step();
    s[0] as f32 * x + s[2] as f32 * z > 0.0
}

/// `RespawnAnchorBlock.RESPAWN_OFFSETS`: around the anchor, then below, then above those,
/// then straight up.
fn anchor_stand_up<L: Level + ?Sized>(level: &L, pos: BlockPos) -> Option<[f64; 3]> {
    const H: [[i32; 2]; 8] = [[0, -1], [-1, 0], [0, 1], [1, 0], [-1, -1], [1, -1], [-1, 1], [1, 1]];
    let mut offsets: Vec<[i32; 3]> = H.iter().map(|h| [h[0], 0, h[1]]).collect();
    offsets.extend(H.iter().map(|h| [h[0], -1, h[1]]));
    offsets.extend(H.iter().map(|h| [h[0], 1, h[1]]));
    offsets.push([0, 1, 0]);
    for check in [true, false] {
        for o in &offsets {
            if let Some(v) = safe_dismount(level, BlockPos::new(pos.x + o[0], pos.y + o[1], pos.z + o[2]), check) {
                return Some(v);
            }
        }
    }
    None
}

/// Where a player respawns at its respawn block (`ServerPlayer.findRespawnAndUseSpawnBlock`):
/// beside a bed, or beside a charged anchor that works (spending a charge); a forced point
/// (`/spawnpoint`) is used as it is. `None`: the point is gone (the caller tells the player).
pub(crate) enum RespawnAt {
    /// The position and the angle facing the block.
    Block([f64; 3], f32),
    /// A forced point: the usual free-space search around it.
    Forced,
    Invalid,
}

pub(crate) fn find_respawn(level: &mut RegionLevel, pos: [i32; 3], forced: bool) -> RespawnAt {
    let bp = BlockPos::new(pos[0], pos[1], pos[2]);
    let s = level.block(bp);
    let dim = level.env.dim;
    let facing_block = |v: [f64; 3]| {
        let d = [bp.x as f64 + 0.5 - v[0], bp.z as f64 + 0.5 - v[2]];
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1.0e-4);
        kiln_entity::mob::mth::wrap_degrees(((d[1] / len).atan2(d[0] / len) * 57.295_776_367_187_5 - 90.0) as f32)
    };
    if is_anchor(s) && (forced || kiln_blocks::state::get_int(s, "charges") > 0) && respawn_anchor_works(dim) {
        let Some(v) = anchor_stand_up(level, bp) else { return RespawnAt::Invalid };
        if !forced {
            let charges = kiln_blocks::state::get_int(s, "charges");
            kiln_blocks::set_block(level, bp, kiln_blocks::state::set_int(s, "charges", charges - 1), flags::ALL);
        }
        return RespawnAt::Block(v, facing_block(v));
    }
    if is_bed(s) && test(bed_rule(dim).can_set_spawn, dim, 0) {
        let Some(v) = bed_stand_up(level, bp, facing(s), 0.0) else { return RespawnAt::Invalid };
        return RespawnAt::Block(v, facing_block(v));
    }
    if forced { RespawnAt::Forced } else { RespawnAt::Invalid }
}

/// Whether a main-hand click on `pos` goes to a bed or respawn anchor: the checks of
/// `useItemOn` before the block reacts (reach, not sneaking with something in hand, not a
/// spectator).
pub(crate) fn intercepts(p: &Player, cells: &kiln_region::CellSet<kiln_world::Cell>, env: &crate::blocks::BlockEnv, pos: [i32; 3], face: i32, cursor: [f32; 3]) -> bool {
    use kiln_item::component::EquipmentSlot;
    use kiln_world::Blocks;
    if p.game_mode == 3 || p.awaiting_teleport.is_some() || crate::blocks::direction(face).is_none() {
        return false;
    }
    if !p.can_reach_block(pos, 1.0) || cursor.iter().any(|&c| (c as f64 - 0.5).abs() >= 1.0000001) || pos[1] > env.min_y + env.height - 1 {
        return false;
    }
    let have_something = !p.inv.selected_item().is_empty() || !p.inv.equipped(EquipmentSlot::OffHand).is_empty();
    if p.sneaking && have_something {
        return false;
    }
    cells.get_block(pos[0], pos[1], pos[2]).is_some_and(handles)
}

fn item_name(s: &kiln_item::ItemStack) -> &'static str {
    if s.is_empty() {
        return "minecraft:air";
    }
    kiln_data::builtin_entries("minecraft:item").and_then(|e| e.get(s.item() as usize).copied()).unwrap_or("minecraft:air")
}

/// A main-hand click on a bed or respawn anchor, then the player gets the clicked block and
/// its neighbour back (`handleUseItemOn`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn use_item_on(
    entities: &mut crate::entities::Entities,
    level: &mut RegionLevel,
    players: &mut [&mut Player],
    i: usize,
    pos: [i32; 3],
    face: i32,
    spawns: &mut Vec<crate::entities::Spawn>,
    deaths: &mut Vec<crate::health::Death>,
) {
    use kiln_item::component::EquipmentSlot;
    let bp = BlockPos::new(pos[0], pos[1], pos[2]);
    let (held, offhand) = {
        let p = &players[i];
        (item_name(p.inv.selected_item()), item_name(p.inv.equipped(EquipmentSlot::OffHand)))
    };
    let (_, outcome) = use_block(players[i], level, bp, held, offhand);
    if let Outcome::Explode(at) = outcome {
        // `Level.explode` with `badRespawnPointExplosion`, power 5, fire, block interaction.
        let salt = (pos[0] as u64) << 40 ^ (pos[2] as u64) << 16 ^ pos[1] as u64;
        crate::entities::with_level(entities, level, players, spawns, deaths, salt, |lvl| {
            kiln_entity::explosion::explode(
                lvl,
                None,
                kiln_entity::math::Vec3::new(at[0], at[1], at[2]),
                5.0,
                true,
                kiln_entity::explosion::Interaction::DestroyWithDecay,
            );
        });
    }
    let p = &mut players[i];
    let step = crate::blocks::direction(face).map_or([0; 3], |d| d.step());
    p.resend_block(level, pos);
    p.resend_block(level, [pos[0] + step[0], pos[1] + step[1], pos[2] + step[2]]);
}

impl Sim {
    /// `ServerLevel.updateSleepingPlayerList` for levels whose sleepers changed, with the
    /// "players sleeping" or "sleeping through this night" overlay.
    pub(crate) fn update_sleeping(&mut self) {
        for dim in 0..self.dims.len() {
            let dirty = self.players.values_mut().filter(|p| p.dim == dim).fold(false, |a, p| std::mem::take(&mut p.sleep.list_dirty) | a);
            if !dirty && !std::mem::take(&mut self.sleep_status[dim].dirty) {
                continue;
            }
            let (active, sleeping) = self
                .players
                .values()
                .filter(|p| p.dim == dim && p.game_mode != 3)
                .fold((0, 0), |(a, s), p| (a + 1, s + p.sleep.pos.is_some() as i32));
            let st = &mut self.sleep_status[dim];
            let changed = (st.sleeping > 0 || sleeping > 0) && (st.active != active || st.sleeping != sleeping);
            st.active = active;
            st.sleeping = sleeping;
            if changed {
                self.announce_sleep_status(dim);
            }
        }
    }

    fn sleep_percentage(&self) -> i32 {
        self.rule_int("minecraft:players_sleeping_percentage")
    }

    /// `ServerLevel.announceSleepStatus`.
    fn announce_sleep_status(&mut self, dim: DimId) {
        let pct = self.sleep_percentage();
        if pct > 100 {
            return;
        }
        let st = self.sleep_status[dim];
        let text = if st.sleeping >= st.needed(pct) {
            kiln_command::tr!("sleep.skipping_night")
        } else {
            kiln_command::tr!("sleep.players_sleeping", st.sleeping, st.needed(pct))
        };
        self.broadcast_in(dim, packets::system_chat(text.to_nbt(), true));
    }

    /// The sleeping part of `ServerLevel.tick`: once enough players have slept 100 ticks, the
    /// level's clock moves to the wake-up time, everyone wakes and rain stops.
    pub(crate) fn tick_sleep(&mut self) {
        let pct = self.sleep_percentage();
        for dim in 0..self.dims.len() {
            let st = self.sleep_status[dim];
            let needed = st.needed(pct);
            let deep = self.players.values().filter(|p| p.dim == dim && p.sleep.pos.is_some() && p.sleep.counter >= 100).count() as i32;
            if st.sleeping < needed || deep < needed {
                continue;
            }
            if self.rule_bool("minecraft:advance_time") && dim == OVERWORLD_ID {
                if let Some(to) = crate::weather::marker_move(self.day_time, 0) {
                    self.day_time = to;
                    self.clock_runs[0].partial = 0.0;
                }
                let pkt = self.time_packet();
                self.broadcast(pkt);
            }
            self.wake_up_all(dim);
            if self.rule_bool("minecraft:advance_weather") && self.is_raining(dim) {
                self.weather.reset_cycle();
            }
        }
    }

    /// `ServerLevel.wakeUpAllPlayers`.
    fn wake_up_all(&mut self, dim: DimId) {
        self.sleep_status[dim].sleeping = 0;
        let mut sleepers: Vec<_> = self.players.values().filter(|p| p.dim == dim && p.sleep.pos.is_some()).map(|p| p.conn).collect();
        sleepers.sort_unstable();
        for conn in sleepers {
            // Out of the map while its region's level runs (the level lists the others).
            let Some(mut p) = self.players.remove(&conn) else { continue };
            let pos = p.pos.map(|c| c.floor() as i32);
            if self.with_level_in(dim, pos, |level| stop_sleep_in_bed(&mut p, level, false, false)).is_none() {
                p.sleep.pos = None;
            }
            self.players.insert(conn, p);
        }
    }
}

/// `SleepStatus` of a level.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SleepStatus {
    pub active: i32,
    pub sleeping: i32,
    /// Players joined or left: recount.
    pub dirty: bool,
}

impl SleepStatus {
    /// `sleepersNeeded`.
    pub fn needed(&self, pct: i32) -> i32 {
        // `Mth.ceil`.
        let f = self.active as f32 * pct as f32 / 100.0;
        let i = f as i32;
        1.max(if f > i as f32 { i + 1 } else { i })
    }
}

/// The saved `minecraft:time_since_rest` of a player (its statistics file).
pub(crate) fn load_time_since_rest(world: &std::path::Path, uuid: uuid::Uuid) -> i32 {
    let path = world.join("players/stats").join(format!("{uuid}.json"));
    let Ok(text) = std::fs::read_to_string(path) else { return 0 };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return 0 };
    json.pointer("/stats/minecraft:custom/minecraft:time_since_rest").and_then(|v| v.as_i64()).unwrap_or(0) as i32
}

/// Writes `minecraft:time_since_rest` into the player's statistics file, keeping the rest.
pub(crate) fn save_time_since_rest(world: &std::path::Path, uuid: uuid::Uuid, value: i32) -> std::io::Result<()> {
    let dir = world.join("players/stats");
    let path = dir.join(format!("{uuid}.json"));
    let mut json = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({ "stats": {}, "DataVersion": kiln_storage::anvil::DATA_VERSION }));
    let custom = json
        .as_object_mut()
        .unwrap()
        .entry("stats")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .map(|s| s.entry("minecraft:custom").or_insert_with(|| serde_json::json!({})));
    if let Some(serde_json::Value::Object(c)) = custom {
        c.insert("minecraft:time_since_rest".into(), serde_json::json!(value));
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(path, serde_json::to_string(&json).unwrap_or_default())
}
