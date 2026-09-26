//! HUD packets (play, clientbound): boss bars, titles, the action bar and the tab list header
//! and footer. Text is a chat component (network NBT); see [`crate::nbt::text`].

use super::packet;
use crate::WriteExt;
use crate::nbt::Tag;
use bytes::{BufMut, Bytes};
use kiln_data::packets::play::clientbound as ids;
use uuid::Uuid;

/// `BossEvent.BossBarColor`, by ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BossBarColor {
    Pink,
    Blue,
    Red,
    Green,
    Yellow,
    Purple,
    White,
}

/// `BossEvent.BossBarOverlay`: a plain bar or one divided into segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BossBarOverlay {
    Progress,
    Notched6,
    Notched10,
    Notched12,
    Notched20,
}

/// Boss bar property bits.
pub mod boss_flags {
    pub const DARKEN_SCREEN: u8 = 0x01;
    pub const PLAY_BOSS_MUSIC: u8 = 0x02;
    pub const CREATE_WORLD_FOG: u8 = 0x04;
}

/// A boss bar operation; the client keys bars by UUID.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BossEvent<'a> {
    Add { name: &'a Tag, progress: f32, color: BossBarColor, overlay: BossBarOverlay, flags: u8 },
    Remove,
    /// Fill fraction, 0.0..=1.0.
    Progress(f32),
    Name(&'a Tag),
    Style { color: BossBarColor, overlay: BossBarOverlay },
    /// [`boss_flags`] bits.
    Flags(u8),
}

pub fn boss_event(id: Uuid, op: &BossEvent) -> Bytes {
    let mut b = packet(ids::BOSS_EVENT);
    b.put_uuid(id);
    match *op {
        BossEvent::Add { name, progress, color, overlay, flags } => {
            b.put_varint(0);
            name.write_network(&mut b);
            b.put_f32(progress);
            b.put_varint(color as i32);
            b.put_varint(overlay as i32);
            b.put_u8(flags);
        }
        BossEvent::Remove => b.put_varint(1),
        BossEvent::Progress(progress) => {
            b.put_varint(2);
            b.put_f32(progress);
        }
        BossEvent::Name(name) => {
            b.put_varint(3);
            name.write_network(&mut b);
        }
        BossEvent::Style { color, overlay } => {
            b.put_varint(4);
            b.put_varint(color as i32);
            b.put_varint(overlay as i32);
        }
        BossEvent::Flags(flags) => {
            b.put_varint(5);
            b.put_u8(flags);
        }
    }
    b.freeze()
}

fn text_packet(id: i32, text: &Tag) -> Bytes {
    let mut b = packet(id);
    text.write_network(&mut b);
    b.freeze()
}

/// Shows a title with the current (or default) timing; see [`set_titles_animation`].
pub fn set_title_text(text: &Tag) -> Bytes {
    text_packet(ids::SET_TITLE_TEXT, text)
}

/// Sets the subtitle shown with the next title.
pub fn set_subtitle_text(text: &Tag) -> Bytes {
    text_packet(ids::SET_SUBTITLE_TEXT, text)
}

pub fn set_action_bar_text(text: &Tag) -> Bytes {
    text_packet(ids::SET_ACTION_BAR_TEXT, text)
}

/// Title timing in ticks (vanilla default 10, 70, 20).
pub fn set_titles_animation(fade_in: i32, stay: i32, fade_out: i32) -> Bytes {
    let mut b = packet(ids::SET_TITLES_ANIMATION);
    b.put_i32(fade_in);
    b.put_i32(stay);
    b.put_i32(fade_out);
    b.freeze()
}

/// Hides the title; `reset_times` also restores the default timing and clears the subtitle.
pub fn clear_titles(reset_times: bool) -> Bytes {
    let mut b = packet(ids::CLEAR_TITLES);
    b.put_bool(reset_times);
    b.freeze()
}

/// Player list header and footer; an empty text (`text("")`) hides one.
pub fn tab_list(header: &Tag, footer: &Tag) -> Bytes {
    let mut b = packet(ids::TAB_LIST);
    header.write_network(&mut b);
    footer.write_network(&mut b);
    b.freeze()
}
