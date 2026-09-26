//! Scoreboard and team packets (play, clientbound).
//!
//! A sidebar: [`set_objective`] with [`ObjectiveMethod::Add`], [`set_display_objective`] with
//! [`display_slot::SIDEBAR`], then one [`set_score`] per line. Teams carry name tag
//! visibility, collision, color and prefix/suffix; [`set_player_team`] adds players by name
//! (or entity UUID string for non-players).

use super::packet;
use crate::WriteExt;
use crate::nbt::Tag;
use bytes::{BufMut, Bytes, BytesMut};
use kiln_data::packets::play::clientbound as ids;

/// How a score is drawn (`NumberFormat`); `None` in the packets means the client's default
/// (red numbers in the sidebar).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumberFormat<'a> {
    /// Nothing is drawn.
    Blank,
    /// The number, with a style: a compound of style keys (`color`, `bold`, ...), e.g.
    /// `{color: "gold"}`. An empty compound is the plain style.
    Styled(&'a Tag),
    /// This text instead of the number.
    Fixed(&'a Tag),
}

impl NumberFormat<'_> {
    /// `minecraft:number_format_type` registry id, then the type's body.
    fn write(&self, b: &mut BytesMut) {
        match *self {
            NumberFormat::Blank => b.put_varint(0),
            NumberFormat::Styled(style) => {
                b.put_varint(1);
                style.write_network(b);
            }
            NumberFormat::Fixed(text) => {
                b.put_varint(2);
                text.write_network(b);
            }
        }
    }
}

fn put_optional_format(b: &mut BytesMut, format: Option<&NumberFormat>) {
    b.put_bool(format.is_some());
    if let Some(f) = format {
        f.write(b);
    }
}

/// `ObjectiveCriteria.RenderType`: how the tab list shows the objective.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderType {
    Integer,
    Hearts,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Objective<'a> {
    pub display_name: &'a Tag,
    pub render_type: RenderType,
    /// Default format for the objective's scores.
    pub number_format: Option<NumberFormat<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ObjectiveMethod<'a> {
    Add(Objective<'a>),
    Remove,
    Change(Objective<'a>),
}

pub fn set_objective(name: &str, method: &ObjectiveMethod) -> Bytes {
    let mut b = packet(ids::SET_OBJECTIVE);
    b.put_string(name);
    let (code, objective) = match method {
        ObjectiveMethod::Add(o) => (0, Some(o)),
        ObjectiveMethod::Remove => (1, None),
        ObjectiveMethod::Change(o) => (2, Some(o)),
    };
    b.put_i8(code);
    if let Some(o) = objective {
        o.display_name.write_network(&mut b);
        b.put_varint(o.render_type as i32);
        put_optional_format(&mut b, o.number_format.as_ref());
    }
    b.freeze()
}

/// Sets `owner`'s score; `display` replaces the owner's name in the sidebar, `format` the
/// objective's number format for this score.
pub fn set_score(owner: &str, objective: &str, score: i32, display: Option<&Tag>, format: Option<&NumberFormat>) -> Bytes {
    let mut b = packet(ids::SET_SCORE);
    b.put_string(owner);
    b.put_string(objective);
    b.put_varint(score);
    b.put_bool(display.is_some());
    if let Some(d) = display {
        d.write_network(&mut b);
    }
    put_optional_format(&mut b, format);
    b.freeze()
}

/// Removes `owner`'s score from `objective`, or from every objective.
pub fn reset_score(owner: &str, objective: Option<&str>) -> Bytes {
    let mut b = packet(ids::RESET_SCORE);
    b.put_string(owner);
    b.put_bool(objective.is_some());
    if let Some(o) = objective {
        b.put_string(o);
    }
    b.freeze()
}

/// `DisplaySlot` ids.
pub mod display_slot {
    use super::TeamColor;

    pub const LIST: i32 = 0;
    pub const SIDEBAR: i32 = 1;
    pub const BELOW_NAME: i32 = 2;

    /// The sidebar shown only to members of teams with this color.
    pub const fn team_sidebar(color: TeamColor) -> i32 {
        3 + color as i32
    }
}

/// Shows `objective` in a [`display_slot`]; an empty name clears the slot.
pub fn set_display_objective(slot: i32, objective: &str) -> Bytes {
    let mut b = packet(ids::SET_DISPLAY_OBJECTIVE);
    b.put_varint(slot);
    b.put_string(objective);
    b.freeze()
}

/// `TeamColor`: the sixteen chat colors, in `ChatFormatting` order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamColor {
    Black,
    DarkBlue,
    DarkGreen,
    DarkAqua,
    DarkRed,
    DarkPurple,
    Gold,
    Gray,
    DarkGray,
    Blue,
    Green,
    Aqua,
    Red,
    LightPurple,
    Yellow,
    White,
}

/// `Team.Visibility` (name tags) and `Team.CollisionRule` share this shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TeamRule {
    #[default]
    Always,
    Never,
    /// Name tags: hidden for other teams. Collision: push other teams only.
    OtherTeams,
    /// Name tags: hidden for the own team. Collision: push the own team only.
    OwnTeam,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TeamParameters<'a> {
    pub display_name: &'a Tag,
    pub prefix: &'a Tag,
    pub suffix: &'a Tag,
    pub name_tag_visibility: TeamRule,
    pub collision_rule: TeamRule,
    /// Colors member names; `None` leaves them uncolored.
    pub color: Option<TeamColor>,
    pub friendly_fire: bool,
    pub see_friendly_invisibles: bool,
}

impl TeamParameters<'_> {
    fn write(&self, b: &mut BytesMut) {
        self.display_name.write_network(b);
        self.prefix.write_network(b);
        self.suffix.write_network(b);
        b.put_varint(self.name_tag_visibility as i32);
        b.put_varint(self.collision_rule as i32);
        b.put_bool(self.color.is_some());
        if let Some(c) = self.color {
            b.put_varint(c as i32);
        }
        b.put_u8(self.friendly_fire as u8 | (self.see_friendly_invisibles as u8) << 1);
    }
}

/// `members` are player names, or entity UUIDs in string form.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TeamMethod<'a> {
    Add(TeamParameters<'a>, &'a [&'a str]),
    Remove,
    Change(TeamParameters<'a>),
    Join(&'a [&'a str]),
    Leave(&'a [&'a str]),
}

pub fn set_player_team(name: &str, method: &TeamMethod) -> Bytes {
    let mut b = packet(ids::SET_PLAYER_TEAM);
    b.put_string(name);
    let (code, params, members): (i8, _, Option<&[&str]>) = match method {
        TeamMethod::Add(p, m) => (0, Some(p), Some(m)),
        TeamMethod::Remove => (1, None, None),
        TeamMethod::Change(p) => (2, Some(p), None),
        TeamMethod::Join(m) => (3, None, Some(m)),
        TeamMethod::Leave(m) => (4, None, Some(m)),
    };
    b.put_i8(code);
    if let Some(p) = params {
        p.write(&mut b);
    }
    if let Some(members) = members {
        b.put_varint(members.len() as i32);
        for m in members {
            b.put_string(m);
        }
    }
    b.freeze()
}
