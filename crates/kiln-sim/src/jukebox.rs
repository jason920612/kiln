//! Jukeboxes (`JukeboxBlock`, `JukeboxBlockEntity`): a music disc used on one goes in and plays
//! (level event 1010 carries the song, which is how clients play it and make the parrots around
//! dance), an empty hand takes it out again (1011) and a jukebox that is broken gives its disc
//! back. The disc lives in the block entity as `RecordItem`. Not modelled: the song ending by
//! itself (clients stop their own music), comparators and hoppers reading the disc.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::{Body, Spawn};
use kiln_blocks::{BlockPos, Effect, Level, state};
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

const RECORD: &str = "RecordItem";

pub(crate) fn is_jukebox(s: u16) -> bool {
    logic::block_class(s) == C::JukeboxBlock
}

fn chunk_of(pos: BlockPos) -> kiln_world::ChunkPos {
    kiln_world::ChunkPos::of_block(pos.x, pos.z)
}

/// The disc in the jukebox block entity at `pos`.
fn record(level: &RegionLevel, pos: BlockPos) -> Option<ItemStack> {
    use kiln_world::Blocks;
    let chunk = level.cells.chunk(chunk_of(pos))?;
    let be = chunk.block_entity((pos.x & 15) as usize, pos.y, (pos.z & 15) as usize)?;
    let Tag::Compound(fields) = &be.nbt else { return None };
    let tag = fields.iter().find(|(k, _)| k == RECORD)?.1.clone();
    ItemStack::from_nbt(&tag).ok().filter(|s| !s.is_empty())
}

/// Writes (or clears) the disc of the jukebox block entity at `pos`.
fn store(level: &mut RegionLevel, pos: BlockPos, disc: Option<&ItemStack>) {
    use kiln_world::Blocks;
    let Some(chunk) = level.cells.chunk_mut(chunk_of(pos)) else { return };
    let (x, z) = ((pos.x & 15) as usize, (pos.z & 15) as usize);
    let Some(mut be) = chunk.block_entity(x, pos.y, z).cloned() else { return };
    if let Tag::Compound(fields) = &mut be.nbt {
        fields.retain(|(k, _)| k != RECORD && k != "ticks_since_song_started");
        if let Some(d) = disc {
            fields.push((RECORD.into(), d.to_nbt()));
            fields.push(("ticks_since_song_started".into(), Tag::Long(0)));
        }
    }
    chunk.set_block_entity(x, pos.y, z, be);
}

/// `JukeboxBlock.useItemOn` with a music disc (`jukebox_playable`): into an empty jukebox.
/// `Some(true)`: it went in; `None`: not for the jukebox (a record already in it takes the
/// empty-hand use instead).
pub(crate) fn use_item_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, off_hand: bool) -> Option<bool> {
    if !is_jukebox(s) || state::get_bool(s, "has_record") {
        return None;
    }
    let stack = p.in_hand(off_hand).clone();
    let playable = stack.get(kiln_item::keys::JUKEBOX_PLAYABLE)?;
    let kiln_item::holder::Holder::Reference(song) = &playable.0 else { return None };
    let song = *song;
    let mut disc = stack.clone();
    disc.set_count(1);
    store(level, pos, Some(&disc));
    kiln_blocks::set_block_and_update(level, pos, state::set_bool(s, "has_record", true));
    level.effect(Effect::GameEvent { pos, event: "minecraft:jukebox_play" });
    level.effect(Effect::LevelEvent { id: 1010, pos, data: song });
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
        p.inv.times_changed += 1;
    }
    Some(true)
}

/// `JukeboxBlock.useWithoutItem`: a jukebox with a record pops it out.
pub(crate) fn use_without_item(level: &mut RegionLevel, pos: BlockPos, spawns: &mut Vec<Spawn>) -> bool {
    let s = level.block(pos);
    if !is_jukebox(s) || !state::get_bool(s, "has_record") {
        return false;
    }
    if let Some(disc) = record(level, pos) {
        store(level, pos, None);
        pop_out(level, pos, disc, spawns);
    }
    kiln_blocks::set_block_and_update(level, pos, state::set_bool(s, "has_record", false));
    true
}

/// `JukeboxBlockEntity.popOutTheItem`: the disc lands above the block, a little off its middle
/// (`Vec3.offsetRandomXZ(random, 0.7)`), and the music stops.
fn pop_out(level: &mut RegionLevel, pos: BlockPos, disc: ItemStack, spawns: &mut Vec<Spawn>) {
    let r = level.random();
    let dx = (r.next_float() - r.next_float()) * 0.7;
    let dz = (r.next_float() - r.next_float()) * 0.7;
    let at = [pos.x as f64 + 0.5 + dx as f64, pos.y as f64 + 1.01, pos.z as f64 + 0.5 + dz as f64];
    let mut spawn = crate::mobs::drop_item(disc, at, (pos.x as u64) << 32 ^ pos.z as u64);
    spawn.pos = at;
    if let Body::Item { pickup_delay, .. } = &mut spawn.body {
        *pickup_delay = 10;
    }
    spawns.push(spawn);
    level.effect(Effect::GameEvent { pos, event: "minecraft:jukebox_stop_play" });
    level.effect(Effect::LevelEvent { id: 1011, pos, data: 0 });
}

/// A block change at `pos` is about to replace a jukebox that holds a disc with a different
/// block: the disc, taken out (the block entity goes with the block).
pub(crate) fn before_removal(level: &RegionLevel, pos: BlockPos, new_state: u16) -> Option<ItemStack> {
    let old = level.block(pos);
    if !is_jukebox(old) || is_jukebox(new_state) || !state::get_bool(old, "has_record") {
        return None;
    }
    record(level, pos)
}

/// The disc a broken jukebox gave up pops out.
pub(crate) fn removed(level: &mut RegionLevel, pos: BlockPos, disc: ItemStack) {
    let mut spawns = Vec::new();
    pop_out(level, pos, disc, &mut spawns);
    level.out.spawns.extend(spawns);
}
