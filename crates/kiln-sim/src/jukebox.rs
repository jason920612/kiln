//! Jukeboxes (`JukeboxBlock`, `JukeboxBlockEntity`, `JukeboxSongPlayer`): a music disc used on one
//! (or put in by a hopper or dispenser) goes in and starts its song (level event 1010 carries the
//! song, which is how clients play it and make the parrots around dance), the song ends by
//! itself once its length and a second more have passed (the disc stays), an empty hand (or a
//! hopper under it) takes the disc out again (1011), and a jukebox that is broken gives its disc
//! back. While a song plays the jukebox powers its neighbours (15) and every 20 ticks makes the
//! `jukebox_play` game event and a note particle; a comparator reads the disc's song
//! (`comparator_output`), playing or not.
//!
//! The disc lives in the block entity as a [`ContainerBe`] (one slot, so hoppers deal with it as
//! with any container) and is saved as `RecordItem`, the song's progress as
//! `ticks_since_song_started`. The `has_record` block state follows the item
//! (`notifyItemChangedInJukebox`).

use crate::Player;
use crate::blocks::RegionLevel;
use crate::container::{BeKind, ContainerBe};
use crate::entities::{Body, Spawn};
use kiln_blocks::{BlockId, BlockPos, Effect, Level, state};
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;

pub(crate) fn is_jukebox(s: u16) -> bool {
    logic::block_class(s) == C::JukeboxBlock
}

/// `JukeboxSong.fromStack` for a song of the registry: the `minecraft:jukebox_song` network id
/// the disc's `jukebox_playable` names (inline songs are not played).
pub(crate) fn song_id(stack: &ItemStack) -> Option<i32> {
    match &stack.get(kiln_item::keys::JUKEBOX_PLAYABLE)?.0 {
        kiln_item::holder::Holder::Reference(id) => Some(*id),
        _ => None,
    }
}

/// The song `id` as the datapack defines it (`None` without datapack: such a song never ends).
fn song_info(level: &RegionLevel, id: i32) -> Option<kiln_loot::JukeboxSong> {
    level.env.loot.as_ref()?.jukebox_song(id)
}

/// `JukeboxBlockEntity.getComparatorOutput`: the disc's song's output (0 for no disc).
pub(crate) fn comparator_output(level: &RegionLevel, c: &ContainerBe) -> i32 {
    c.items.first().and_then(song_id).and_then(|id| song_info(level, id)).map_or(0, |s| s.comparator_output)
}

/// A jukebox at `pos` plays a song (`JukeboxSongPlayer.isPlaying`). A song read from the saved
/// data that is over by now was never started (`setSongWithoutPlaying`).
pub(crate) fn is_playing(level: &RegionLevel, pos: BlockPos) -> bool {
    level.blocks.containers.get(pos).is_some_and(|c| {
        c.kind == BeKind::Jukebox && c.song.is_some_and(|(id, ticks)| !(c.song_unchecked && song_info(level, id).is_some_and(|i| i.has_finished(ticks))))
    })
}

/// The state of an empty jukebox block, for game events about a block that is gone.
fn jukebox_state() -> u16 {
    kiln_data::blocks_types::block_by_name("minecraft:jukebox").map_or(0, |b| b.default)
}

/// `BlockEntity.setChanged(level, pos, state)`: the chunk is to be saved and comparators read
/// the jukebox again.
fn set_changed(level: &mut RegionLevel, pos: BlockPos) {
    if let Some(c) = level.blocks.containers.get_mut(pos) {
        c.mark_changed();
    }
    crate::container::hopper::changed(level, pos);
}

/// `JukeboxBlockEntity.onSongChanged` (`JukeboxSongPlayer.OnSongChanged.notifyChange`): the
/// neighbours (a redstone lamp next to it) and comparators are told.
fn song_changed(level: &mut RegionLevel, pos: BlockPos) {
    kiln_blocks::update::update_neighbors_at(level, pos, BlockId::of(jukebox_state()));
    set_changed(level, pos);
}

/// `JukeboxSongPlayer.play`: the song starts from its first tick, and the clients are told.
fn play(level: &mut RegionLevel, pos: BlockPos, id: i32) {
    if let Some(c) = level.blocks.containers.get_mut(pos) {
        c.song = Some((id, 0));
        c.song_unchecked = false;
    }
    level.effect(Effect::LevelEvent { id: 1010, pos, data: id });
    song_changed(level, pos);
}

/// `JukeboxSongPlayer.stop`: the music stops (nothing happens when none played).
fn stop(level: &mut RegionLevel, pos: BlockPos, state_of_block: u16) {
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    if c.song.take().is_none() {
        return;
    }
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:jukebox_stop_play", state: state_of_block });
    level.effect(Effect::LevelEvent { id: 1011, pos, data: 0 });
    song_changed(level, pos);
}

/// `JukeboxBlockEntity.setTheItem` once the item is in the slot: `has_record` follows it
/// (`notifyItemChangedInJukebox`), then its song starts or the music stops.
pub(crate) fn item_changed(level: &mut RegionLevel, pos: BlockPos) {
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    c.item_changed = false;
    let disc = c.items.first().cloned().unwrap_or_else(ItemStack::empty);
    let has_item = !disc.is_empty();
    let s = level.block(pos);
    if is_jukebox(s) {
        kiln_blocks::set_block(level, pos, state::set_bool(s, "has_record", has_item), kiln_blocks::flags::CLIENTS);
        let now = level.block(pos);
        level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: now });
    }
    match song_id(&disc) {
        Some(id) if has_item => play(level, pos, id),
        _ => {
            let now = level.block(pos);
            stop(level, pos, now);
        }
    }
}

/// A hopper or dispenser changed the jukebox's item: what `setTheItem` does follows.
pub(crate) fn settle(level: &mut RegionLevel, pos: BlockPos) {
    if level.blocks.containers.get(pos).is_some_and(|c| c.kind == BeKind::Jukebox && c.item_changed) {
        item_changed(level, pos);
    }
}

/// `JukeboxBlockEntity.tick` (`JukeboxSongPlayer.tick`), for a jukebox that holds a record: the
/// song ends when it has run its length and a second more; every 20 ticks it is heard (a
/// `jukebox_play` game event) and seen (a note).
pub(crate) fn tick(level: &mut RegionLevel, pos: BlockPos) {
    let s = level.block(pos);
    if !is_jukebox(s) || !state::get_bool(s, "has_record") {
        return;
    }
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    // A song read from the saved data is checked against its length now that the data is at hand
    // (`setSongWithoutPlaying` does not start a song that is over).
    if std::mem::take(&mut c.song_unchecked)
        && let Some((id, ticks)) = c.song
        && level.env.loot.as_ref().and_then(|l| l.jukebox_song(id)).is_some_and(|i| i.has_finished(ticks))
    {
        if let Some(c) = level.blocks.containers.get_mut(pos) {
            c.song = None;
        }
    }
    let Some((id, ticks)) = level.blocks.containers.get(pos).and_then(|c| c.song) else { return };
    if song_info(level, id).is_some_and(|i| i.has_finished(ticks)) {
        stop(level, pos, s);
        return;
    }
    if ticks % 20 == 0 {
        level.effect(Effect::BlockGameEvent { pos, event: "minecraft:jukebox_play", state: s });
        let color = level.random().next_int_bounded(4) as f32 / 24.0;
        level.effect(Effect::MusicNote { pos, color });
    }
    if let Some(c) = level.blocks.containers.get_mut(pos)
        && let Some(song) = c.song.as_mut()
    {
        song.1 += 1;
    }
}

/// `JukeboxBlock.useItemOn` with a music disc (`JukeboxPlayable.tryInsertIntoJukebox`): into an
/// empty jukebox. `Some(true)`: it went in; `None`: not for the jukebox (a record already in it
/// takes the empty-hand use instead).
pub(crate) fn use_item_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, s: u16, off_hand: bool) -> Option<bool> {
    if !is_jukebox(s) || state::get_bool(s, "has_record") {
        return None;
    }
    let stack = p.in_hand(off_hand).clone();
    stack.get(kiln_item::keys::JUKEBOX_PLAYABLE)?;
    let mut disc = stack.clone();
    disc.set_count(1);
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
        p.inv.times_changed += 1;
    }
    if let Some(c) = level.blocks.containers.get_mut(pos).filter(|c| c.kind == BeKind::Jukebox) {
        // `setTheItem`.
        c.items[0] = disc;
        c.mark_changed();
        item_changed(level, pos);
        level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: s });
    }
    p.award_stat(*crate::player_stats::stat::PLAY_RECORD, 1);
    Some(true)
}

/// `JukeboxBlock.useWithoutItem`: a jukebox with a record pops it out.
pub(crate) fn use_without_item(level: &mut RegionLevel, pos: BlockPos, spawns: &mut Vec<Spawn>) -> bool {
    let s = level.block(pos);
    if !is_jukebox(s) || !state::get_bool(s, "has_record") {
        return false;
    }
    let Some(c) = level.blocks.containers.get_mut(pos).filter(|c| c.kind == BeKind::Jukebox) else { return false };
    // `popOutTheItem`: `removeTheItem` (the state and the music follow it), the disc lands above
    // the block, then `onSongChanged`.
    if c.items[0].is_empty() {
        return true;
    }
    let disc = std::mem::replace(&mut c.items[0], ItemStack::empty());
    c.item_changed = true;
    c.mark_changed();
    item_changed(level, pos);
    spawns.push(pop_out(level, pos, disc));
    song_changed(level, pos);
    true
}

/// The item entity `popOutTheItem` makes: above the block, a little off its middle
/// (`Vec3.offsetRandomXZ(random, 0.7)`: each axis `(nextFloat - 0.5) * 0.7`).
fn pop_out(level: &mut RegionLevel, pos: BlockPos, disc: ItemStack) -> Spawn {
    let r = level.random();
    let dx = (r.next_float() - 0.5) * 0.7f32;
    let dz = (r.next_float() - 0.5) * 0.7f32;
    let at = [pos.x as f64 + 0.5 + dx as f64, pos.y as f64 + 1.01, pos.z as f64 + 0.5 + dz as f64];
    let mut spawn = crate::mobs::drop_item(disc, at, (pos.x as u64) << 32 ^ pos.z as u64);
    spawn.pos = at;
    if let Body::Item { pickup_delay, .. } = &mut spawn.body {
        *pickup_delay = 10;
    }
    spawn
}

/// `Clearable.tryClear` of a jukebox about to be replaced by a command: the disc is deleted
/// (`removeTheItem`) and the music stops, nothing drops.
pub(crate) fn cleared(level: &mut RegionLevel, pos: BlockPos, removed: &mut ContainerBe) {
    removed.items[0] = ItemStack::empty();
    if removed.song.take().is_none() {
        return;
    }
    let jukebox = jukebox_state();
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:jukebox_stop_play", state: jukebox });
    level.effect(Effect::LevelEvent { id: 1011, pos, data: 0 });
    kiln_blocks::update::update_neighbors_at(level, pos, BlockId::of(jukebox));
    crate::container::hopper::changed(level, pos);
}

/// `JukeboxBlockEntity.preRemoveSideEffects`: the block was replaced and the block entity is
/// `removed`: its disc pops out (the music stops and the neighbours are told, the block state is
/// no longer the jukebox's to set).
pub(crate) fn removed(level: &mut RegionLevel, pos: BlockPos, removed: &mut ContainerBe) {
    let disc = std::mem::replace(&mut removed.items[0], ItemStack::empty());
    if disc.is_empty() {
        return;
    }
    // `removeTheItem` -> `setTheItem(EMPTY)` -> `stop` (the block entity is out of the level, so
    // the stop is done on its own song).
    let stopped = removed.song.take().is_some();
    let jukebox = jukebox_state();
    if stopped {
        level.effect(Effect::BlockGameEvent { pos, event: "minecraft:jukebox_stop_play", state: jukebox });
        level.effect(Effect::LevelEvent { id: 1011, pos, data: 0 });
        kiln_blocks::update::update_neighbors_at(level, pos, BlockId::of(jukebox));
        crate::container::hopper::changed(level, pos);
    }
    let spawn = pop_out(level, pos, disc);
    level.out.spawns.push(spawn);
    kiln_blocks::update::update_neighbors_at(level, pos, BlockId::of(jukebox));
    crate::container::hopper::changed(level, pos);
}
