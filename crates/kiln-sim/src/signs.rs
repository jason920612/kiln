//! Signs (standing, wall, hanging and wall hanging): `SignBlock.useItemOn` and `useWithoutItem`
//! (dyes, glow ink, ink sacs and honeycomb on the side the player faces; waxed signs refuse;
//! click events; the text editor), `SignBlock.setPlacedBy` (the editor opens for the placer),
//! `ServerGamePacketListenerImpl.handleSignUpdate` / `SignBlockEntity.updateSignText` (the
//! editor's lines, only from the player the sign let in) and the editing lock
//! (`playerWhoMayEdit`, cleared when its holder walks away: `SignBlockEntity.tick`).
//!
//! The text lives in the block entity's NBT (`front_text`, `back_text`, `is_waxed`,
//! `allow_op_features`) as vanilla saves it. Not simulated: running the commands of a click
//! event on a sign with `allow_op_features` (the click counts as handled, as it does without).

use crate::Player;
use crate::blocks::RegionLevel;
use kiln_blocks::level::Effect;
use kiln_blocks::{BlockPos, Level, state};
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_item::component::{DyeColor, SignText};
use kiln_item::{ComponentValue, ItemStack, Text, Value, keys};
use kiln_link::ConnId;
use kiln_proto::nbt::Tag;
use kiln_proto::packets;
use kiln_proto::packets::world_fx;
use kiln_world::Blocks;
use std::collections::HashMap;

/// Players that may edit a sign (`SignBlockEntity.playerWhoMayEdit`), by connection.
#[derive(Default)]
pub struct SignEditors {
    map: HashMap<BlockPos, ConnId>,
}

impl SignEditors {
    pub fn get(&self, pos: BlockPos) -> Option<ConnId> {
        self.map.get(&pos).copied()
    }

    pub fn set(&mut self, pos: BlockPos, conn: ConnId) {
        self.map.insert(pos, conn);
    }

    pub fn clear(&mut self, pos: BlockPos) {
        self.map.remove(&pos);
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// The region split: each lock goes to the part owning its chunk.
    pub fn split_into(&mut self, parts: &mut [&mut SignEditors], owner: impl Fn(BlockPos) -> usize) {
        for (p, c) in self.map.drain() {
            parts[owner(p)].map.insert(p, c);
        }
    }

    pub fn merge(&mut self, from: SignEditors) {
        self.map.extend(from.map);
    }
}

/// Whether the block is one of the sign blocks.
pub(crate) fn is_sign(s: u16) -> bool {
    logic::is_instance(s, C::SignBlock)
}

fn is_hanging(s: u16) -> bool {
    matches!(logic::block_class(s), C::CeilingHangingSignBlock | C::WallHangingSignBlock)
}

// ---- the block entity -----------------------------------------------------------------------

/// A sign block entity's data.
#[derive(Clone, Default)]
pub(crate) struct Sign {
    front: SignText,
    back: SignText,
    waxed: bool,
    allow_op: bool,
}

fn side_from(tag: Option<&Tag>) -> SignText {
    tag.and_then(|t| SignText::from_value(&Value::from_nbt(t)).ok()).unwrap_or_default()
}

/// A sign's data as vanilla saves it after loading it: both sides parsed and written whole
/// (color, glow, four messages), whatever fields the loaded data had.
pub(crate) fn canonical(fields: &mut [(String, Tag)]) {
    for (k, v) in fields.iter_mut() {
        if k == "front_text" || k == "back_text" {
            *v = side_from(Some(v)).to_value().to_nbt();
        }
    }
}

fn chunk_pos(pos: BlockPos) -> kiln_world::ChunkPos {
    kiln_world::ChunkPos::of_block(pos.x, pos.z)
}

fn load(level: &RegionLevel, pos: BlockPos) -> Option<Sign> {
    let chunk = level.cells.chunk(chunk_pos(pos))?;
    let nbt = match chunk.block_entity((pos.x & 15) as usize, pos.y, (pos.z & 15) as usize) {
        Some(be) => &be.nbt,
        None => return Some(Sign::default()),
    };
    Some(Sign {
        front: side_from(nbt.get("front_text")),
        back: side_from(nbt.get("back_text")),
        waxed: nbt.get("is_waxed").and_then(Tag::as_i64).is_some_and(|v| v != 0),
        allow_op: nbt.get("allow_op_features").and_then(Tag::as_i64).is_some_and(|v| v != 0),
    })
}

/// `SignBlockEntity.saveAdditional` into the chunk's block entity, and the block update that
/// sends it to players (`markUpdated`).
fn store(level: &mut RegionLevel, pos: BlockPos, sign: &Sign) {
    let state = level.block(pos);
    let Some(kind) = kiln_data::block_props::block_entity_type(state) else { return };
    let Some(chunk) = level.cells.chunk_mut(chunk_pos(pos)) else { return };
    let (x, z) = ((pos.x & 15) as usize, (pos.z & 15) as usize);
    let mut be = chunk.block_entity(x, pos.y, z).cloned().unwrap_or_else(|| kiln_world::block_entity::BlockEntity::new(kind as u16));
    if let Tag::Compound(fields) = &mut be.nbt {
        fields.retain(|(k, _)| !matches!(k.as_str(), "front_text" | "back_text" | "is_waxed" | "allow_op_features"));
        if !fields.iter().any(|(k, _)| k == "components") {
            fields.push(("components".into(), Tag::Compound(Vec::new())));
        }
        if sign.allow_op {
            fields.push(("allow_op_features".into(), Tag::Byte(1)));
        }
        fields.push(("front_text".into(), sign.front.to_value().to_nbt()));
        fields.push(("back_text".into(), sign.back.to_value().to_nbt()));
        fields.push(("is_waxed".into(), Tag::Byte(sign.waxed as i8)));
    }
    chunk.set_block_entity(x, pos.y, z, be);
    level.out.changed.push([pos.x, pos.y, pos.z]);
}

// ---- text components ------------------------------------------------------------------------

/// Keys of a component's contents (the rest of a compound is its style or siblings).
const CONTENT_KEYS: [&str; 7] = ["translate", "score", "selector", "keybind", "nbt", "object", "fallback"];

/// `component.getContents() instanceof PlainTextContents`.
fn is_plain(t: &Tag) -> bool {
    match t {
        Tag::String(_) => true,
        Tag::Compound(f) => !f.iter().any(|(k, _)| CONTENT_KEYS.contains(&k.as_str())),
        Tag::List(l) => l.first().is_none_or(is_plain),
        _ => false,
    }
}

/// `component.getString()`; text that is not literal counts as non-empty (it has no language
/// file here to be looked up in).
fn plain_string(t: &Tag) -> String {
    match t {
        Tag::String(s) => s.clone(),
        Tag::Compound(f) => {
            if !is_plain(t) {
                return "?".to_owned();
            }
            let mut out = f.iter().find(|(k, _)| k == "text").and_then(|(_, v)| v.as_str()).unwrap_or("").to_owned();
            if let Some(Tag::List(extra)) = f.iter().find(|(k, _)| k == "extra").map(|(_, v)| v) {
                for e in extra {
                    out.push_str(&plain_string(e.unwrap_list_element()));
                }
            }
            out
        }
        Tag::List(l) => l.iter().map(|e| plain_string(e.unwrap_list_element())).collect(),
        _ => String::new(),
    }
}

/// The style keys of a component compound (`Style` as `ComponentSerialization` writes it).
const STYLE_KEYS: [&str; 11] = [
    "color", "bold", "italic", "underlined", "strikethrough", "obfuscated", "click_event", "hover_event", "insertion", "font", "shadow_color",
];

/// `component.getStyle()` as NBT fields.
fn style_of(t: &Tag) -> Vec<(String, Tag)> {
    match t {
        Tag::Compound(f) => f.iter().filter(|(k, _)| STYLE_KEYS.contains(&k.as_str())).cloned().collect(),
        Tag::List(l) => l.first().map(|e| style_of(e.unwrap_list_element())).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// `Component.literal(text).setStyle(style)`.
fn literal_with_style(text: &str, style: Vec<(String, Tag)>) -> Text {
    if style.is_empty() {
        return Text::literal(text);
    }
    let mut fields = vec![("text".to_owned(), Tag::String(text.to_owned()))];
    fields.extend(style);
    Text::from_nbt(Tag::Compound(fields)).unwrap_or_else(|| Text::literal(text))
}

/// The action of the click event in a line's style.
fn click_action(t: &Tag) -> Option<String> {
    let style = style_of(t);
    let (_, event) = style.iter().find(|(k, _)| k == "click_event")?;
    event.get("action").and_then(Tag::as_str).map(str::to_owned)
}

/// `SignText.hasMessage`.
fn has_message(t: &SignText) -> bool {
    t.messages.iter().any(|m| !plain_string(m.nbt()).is_empty())
}

/// `SignText.hasEditableText`.
fn has_editable_text(t: &SignText) -> bool {
    t.messages.iter().all(|m| is_plain(m.nbt()))
}

/// `SignText.hasAnyClickCommands`.
#[allow(dead_code)]
fn has_click_commands(t: &SignText) -> bool {
    t.messages.iter().any(|m| click_action(m.nbt()).as_deref() == Some("run_command"))
}

// ---- which side a player faces --------------------------------------------------------------

/// `SignBlock.getYRotationDegrees`.
fn y_rotation(s: u16) -> f32 {
    match logic::block_class(s) {
        // `RotationSegment.convertToDegrees`: the angle in [-180, 180).
        C::StandingSignBlock | C::CeilingHangingSignBlock => {
            let degrees = (state::get_int(s, "rotation") & 15) as f32 * 22.5;
            if degrees >= 180.0 { degrees - 360.0 } else { degrees }
        }
        _ => state::get_dir(s, "facing").map_or(0.0, |d| (d.to_2d() & 3) as f32 * 90.0),
    }
}

/// `SignBlock.getSignHitboxCenterPosition` (x and z): wall signs are the middle of their
/// thin board.
fn hitbox_center(s: u16) -> (f64, f64) {
    use kiln_blocks::Direction as D;
    if logic::block_class(s) != C::WallSignBlock {
        return (0.5, 0.5);
    }
    match state::get_dir(s, "facing") {
        Some(D::North) => (0.5, 0.9375),
        Some(D::South) => (0.5, 0.0625),
        Some(D::East) => (0.0625, 0.5),
        Some(D::West) => (0.9375, 0.5),
        _ => (0.5, 0.5),
    }
}

/// `SignBlockEntity.getSlotPlayerIsFacing`: whether the front is the side facing the player.
fn facing_front(s: u16, pos: BlockPos, player: [f64; 3]) -> bool {
    let (cx, cz) = hitbox_center(s);
    let dx = player[0] - (pos.x as f64 + cx);
    let dz = player[2] - (pos.z as f64 + cz);
    let sign_rot = y_rotation(s);
    let player_rot = (kiln_entity::mob::mth::atan2(dz, dx) * 57.2957763671875) as f32 - 90.0;
    // `Mth.degreesDifferenceAbs`.
    kiln_entity::mob::mth::wrap_degrees(player_rot - sign_rot).abs() <= 90.0
}

// ---- applicators ----------------------------------------------------------------------------

/// `SignApplicator`: the items that change a sign.
#[derive(Clone, Copy)]
enum Applicator {
    Dye(DyeColor),
    GlowInk,
    Ink,
    Honeycomb,
}

fn applicator_of(stack: &ItemStack) -> Option<Applicator> {
    if stack.is_empty() {
        return None;
    }
    match stack.item_name() {
        "minecraft:glow_ink_sac" => return Some(Applicator::GlowInk),
        "minecraft:ink_sac" => return Some(Applicator::Ink),
        "minecraft:honeycomb" => return Some(Applicator::Honeycomb),
        _ => {}
    }
    stack.get(keys::DYE).map(|c| Applicator::Dye(*c))
}

impl Applicator {
    /// `SignApplicator.canApplyToSign`: everything but honeycomb needs some text.
    fn can_apply(self, text: &SignText) -> bool {
        matches!(self, Applicator::Honeycomb) || has_message(text)
    }

    /// `tryApplyToSign`: changes the sign, and plays the sound, when that changes anything.
    fn try_apply(self, level: &mut RegionLevel, pos: BlockPos, sign: &mut Sign, front: bool) -> bool {
        let text = if front { &mut sign.front } else { &mut sign.back };
        let sound = match self {
            Applicator::Dye(color) => {
                if text.color == color {
                    return false;
                }
                text.color = color;
                "minecraft:item.dye.use"
            }
            Applicator::GlowInk => {
                if text.has_glowing_text {
                    return false;
                }
                text.has_glowing_text = true;
                "minecraft:item.glow_ink_sac.use"
            }
            Applicator::Ink => {
                if !text.has_glowing_text {
                    return false;
                }
                text.has_glowing_text = false;
                "minecraft:item.ink_sac.use"
            }
            Applicator::Honeycomb => {
                if sign.waxed {
                    return false;
                }
                sign.waxed = true;
                store(level, pos, sign);
                // The wax-on particles (`LevelEvent.PARTICLES_AND_SOUND_WAX_ON`) and the sound.
                level.effect(Effect::LevelEvent { id: 3003, pos, data: 0 });
                level.effect(Effect::Sound { pos, sound: "minecraft:item.honeycomb.wax_on", volume: 1.0, pitch: 1.0 });
                return true;
            }
        };
        store(level, pos, sign);
        level.effect(Effect::Sound { pos, sound, volume: 1.0, pitch: 1.0 });
        true
    }
}

// ---- interaction ----------------------------------------------------------------------------

/// `Player.mayBuild`.
fn may_build(p: &Player) -> bool {
    p.game_mode <= 1
}

/// `SignBlockEntity.executeClickCommandsIfPresent`: whether a line carried a click event that
/// acts (run a command, show a dialog, a custom action). Without `allow_op_features` nothing
/// runs and the player is told so.
fn execute_click_commands(p: &mut Player, sign: &Sign, front: bool) -> bool {
    let text = if front { &sign.front } else { &sign.back };
    let mut ran = false;
    for m in &text.messages {
        if matches!(click_action(m.nbt()).as_deref(), Some("run_command" | "show_dialog" | "custom")) {
            ran = true;
        }
    }
    if !sign.allow_op && ran {
        let message = Tag::Compound(vec![
            ("translate".into(), Tag::String("sign.click_actions_disabled".into())),
            ("color".into(), Tag::String("red".into())),
        ]);
        p.send(packets::system_chat(message, true));
    }
    ran
}

/// `ServerPlayer.openTextEdit` after `SignBlock.openTextEdit`: the player becomes the one who
/// may edit, gets the sign's block again and the editor.
fn open_text_edit(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, front: bool) {
    level.blocks.sign_editors.set(pos, p.conn);
    p.send(packets::block_update([pos.x, pos.y, pos.z], level.block(pos)));
    p.send(world_fx::open_sign_editor([pos.x, pos.y, pos.z], front));
}

/// `BlockState.useItemOn` then, for the main hand when it passes, `useWithoutItem` of a sign
/// the player clicked. Returns whether the sign took the click (anything else goes on to
/// `Item.useOn`).
pub(crate) fn use_on(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, off_hand: bool) -> bool {
    let s = level.block(pos);
    if !is_sign(s) {
        return false;
    }
    let Some(mut sign) = load(level, pos) else { return false };
    let front = facing_front(s, pos, p.pos);
    let editing_other = level.blocks.sign_editors.get(pos).is_some_and(|c| c != p.conn);
    let held = p.in_hand(off_hand).clone();
    // `SignBlock.useItemOn`.
    if let Some(applicator) = applicator_of(&held).filter(|_| may_build(p))
        && !sign.waxed
        && !editing_other
    {
        let text = if front { &sign.front } else { &sign.back };
        if applicator.can_apply(text) && applicator.try_apply(level, pos, &mut sign, front) {
            execute_click_commands(p, &sign, front);
            p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, held.item()), 1);
            level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_change", state: s });
            // `ItemStack.consume(1, player)`.
            if !p.infinite_materials() {
                let i = p.hand_index(off_hand);
                kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
                p.inv.times_changed += 1;
            }
            return true;
        }
    }
    if off_hand {
        return false;
    }
    // `SignBlock.useWithoutItem`.
    let clicked = execute_click_commands(p, &sign, front);
    if sign.waxed {
        let sound = if is_hanging(s) { "minecraft:block.hanging_sign.waxed_interact_fail" } else { "minecraft:block.sign.waxed_interact_fail" };
        level.effect(Effect::Sound { pos, sound, volume: 1.0, pitch: 1.0 });
        return true;
    }
    if clicked {
        return true;
    }
    let text = if front { &sign.front } else { &sign.back };
    if !editing_other && may_build(p) && has_editable_text(text) {
        open_text_edit(p, level, pos, front);
        return true;
    }
    false
}

/// `SignBlock.setPlacedBy`: the editor opens on the front for the player who placed the sign.
pub(crate) fn placed_by(p: &mut Player, level: &mut RegionLevel, pos: BlockPos) {
    let s = level.block(pos);
    if !is_sign(s) {
        return;
    }
    let Some(sign) = load(level, pos) else { return };
    if !sign.waxed && has_editable_text(&sign.front) {
        open_text_edit(p, level, pos, true);
    }
}

/// `ChatFormatting.stripFormatting`: `§` and one of `0-9a-fk-or` (either case) go.
fn strip_formatting(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{a7}' && chars.peek().is_some_and(|n| matches!(n.to_ascii_lowercase(), '0'..='9' | 'a'..='f' | 'k'..='o' | 'r')) {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// `handleSignUpdate` (with `FilteredText.passThrough`): the editor's lines replace the text of
/// one side, each keeping the style of the line it replaces, if the player is the one the sign
/// let in and the sign is not waxed; the lock is released and players get the new text.
pub(crate) fn update_text(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, lines: &[String; 4], front: bool) {
    let s = level.block(pos);
    if !is_sign(s) {
        return;
    }
    let Some(mut sign) = load(level, pos) else { return };
    if sign.waxed || level.blocks.sign_editors.get(pos) != Some(p.conn) {
        return;
    }
    let text = if front { &mut sign.front } else { &mut sign.back };
    let lines: Vec<Text> = (0..4).map(|i| literal_with_style(&strip_formatting(&lines[i]), style_of(text.messages[i].nbt()))).collect();
    text.messages = std::array::from_fn(|i| lines[i].clone());
    text.filtered_messages = text.messages.clone();
    store(level, pos, &sign);
    level.blocks.sign_editors.clear(pos);
}

/// `SignBlockEntity.tick`: the lock is released when its player is gone or too far from the
/// sign (`isWithinBlockInteractionRange(pos, 4)`).
pub(crate) fn tick(level: &mut RegionLevel) {
    if level.blocks.sign_editors.len() == 0 {
        return;
    }
    let stale: Vec<BlockPos> = level
        .blocks
        .sign_editors
        .map
        .iter()
        .filter(|(pos, conn)| {
            let Some(b) = level.bodies.iter().find(|b| b.conn == Some(**conn)) else { return true };
            let eye = [(b.min[0] + b.max[0]) / 2.0, b.min[1] + 1.62, (b.min[2] + b.max[2]) / 2.0];
            let d2: f64 = [(pos.x, 0), (pos.y, 1), (pos.z, 2)]
                .iter()
                .map(|&(c, i)| {
                    let (lo, hi) = (c as f64, c as f64 + 1.0);
                    (lo - eye[i]).max(0.0).max(eye[i] - hi).powi(2)
                })
                .sum();
            d2 >= (4.5 + 4.0) * (4.5 + 4.0)
        })
        .map(|(p, _)| *p)
        .collect();
    for p in stale {
        level.blocks.sign_editors.clear(p);
    }
}

#[cfg(test)]
impl crate::Sim {
    /// Makes `conn` the player who may edit the sign at an overworld position.
    pub(crate) fn lock_sign(&mut self, conn: ConnId, at: [i32; 3]) {
        let pos = BlockPos::new(at[0], at[1], at[2]);
        let region = self.dims[crate::OVERWORLD_ID].regions.at_mut(chunk_pos(pos).cell()).expect("sign region");
        region.part_mut().1.sign_editors.set(pos, conn);
    }
}
