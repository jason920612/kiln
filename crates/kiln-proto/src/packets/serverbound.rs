//! Types and decoders for serverbound play packets beyond movement and chat. Layouts, limits
//! and fallbacks follow the 26.3 decoders (`net.minecraft.network.protocol.game.Serverbound*`):
//! `readEnum` values out of range are errors, `idMapper` values fall back as vanilla does, and
//! values vanilla clamps on decode are clamped here too.

use super::common::read_enum;
use super::entity::read_lp_vec3;
use super::{PlayIn, read_position};
use crate::{DecodeError, Reader};

/// `InteractionHand`; unknown ids are the main hand, as in vanilla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hand {
    Main,
    Off,
}

fn read_hand(r: &mut Reader) -> Result<Hand, DecodeError> {
    Ok(if r.varint()? == 1 { Hand::Off } else { Hand::Main })
}

/// `client_command` actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientCommand {
    /// The respawn button on the death screen.
    PerformRespawn,
    /// The statistics screen opened.
    RequestStats,
    /// The game rule screen opened (answered only with permission).
    RequestGameRuleValues,
}

/// `CommandBlockEntity.Mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandBlockMode {
    Sequence,
    Auto,
    Redstone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandBlockUpdate {
    pub pos: [i32; 3],
    pub command: String,
    pub mode: CommandBlockMode,
    pub track_output: bool,
    pub conditional: bool,
    /// "Always active" rather than "needs redstone".
    pub automatic: bool,
}

/// `StructureBlockEntity.UpdateType`: what the structure block screen asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureUpdateType {
    UpdateData,
    SaveArea,
    LoadArea,
    ScanArea,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureMode {
    Save,
    Load,
    Corner,
    Data,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mirror {
    None,
    LeftRight,
    FrontBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    None,
    Clockwise90,
    Clockwise180,
    CounterClockwise90,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructureBlockUpdate {
    pub pos: [i32; 3],
    pub update_type: StructureUpdateType,
    pub mode: StructureMode,
    pub name: String,
    /// Clamped to -48..=48.
    pub offset: [i8; 3],
    /// Clamped to 0..=48.
    pub size: [i8; 3],
    pub mirror: Mirror,
    pub rotation: Rotation,
    pub metadata: String,
    /// Clamped to 0.0..=1.0.
    pub integrity: f32,
    pub seed: i64,
    pub ignore_entities: bool,
    pub show_air: bool,
    pub show_bounding_box: bool,
    pub strict: bool,
}

/// `RecipeBookType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipeBookType {
    Crafting,
    Furnace,
    BlastFurnace,
    Smoker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JigsawBlockUpdate {
    pub pos: [i32; 3],
    pub name: String,
    pub target: String,
    pub pool: String,
    pub final_state: String,
    /// `true` for "rollable", `false` for "aligned" (vanilla's fallback for unknown names).
    pub rollable: bool,
    pub selection_priority: i32,
    pub placement_priority: i32,
}

/// Longest sign line vanilla accepts, in UTF-16 units.
pub const MAX_SIGN_LINE: usize = 384;
pub const MAX_BOOK_PAGES: usize = 100;
pub const MAX_BOOK_PAGE: usize = 1024;
pub const MAX_BOOK_TITLE: usize = 32;
/// Longest structure block metadata string vanilla reads.
pub const MAX_STRUCTURE_METADATA: usize = 128;

const MAX_STRING: usize = 32767;

fn string(r: &mut Reader, max: usize) -> Result<String, DecodeError> {
    Ok(r.string(max)?.to_owned())
}

fn optional_varint(r: &mut Reader) -> Result<Option<i32>, DecodeError> {
    Ok(if r.bool()? { Some(r.varint()?) } else { None })
}

pub fn read_client_command(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    use ClientCommand::*;
    Ok(PlayIn::ClientCommand(read_enum(r, &[PerformRespawn, RequestStats, RequestGameRuleValues], "client command")?))
}

pub fn read_interact(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    Ok(PlayIn::Interact {
        entity_id: r.varint()?,
        hand: read_hand(r)?,
        location: read_lp_vec3(r)?,
        sneaking: r.bool()?,
    })
}

pub fn read_use_item(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    Ok(PlayIn::UseItem { hand: read_hand(r)?, sequence: r.varint()?, yaw: r.f32()?, pitch: r.f32()? })
}

pub fn read_sign_update(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    let pos = read_position(r)?;
    let lines = [
        string(r, MAX_SIGN_LINE)?,
        string(r, MAX_SIGN_LINE)?,
        string(r, MAX_SIGN_LINE)?,
        string(r, MAX_SIGN_LINE)?,
    ];
    // SignTextSlot: 0 back, 1 front; unknown ids are the back.
    let front = r.varint()? == 1;
    Ok(PlayIn::SignUpdate { pos, lines: Box::new(lines), front })
}

pub fn read_set_command_block(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    use CommandBlockMode::*;
    let pos = read_position(r)?;
    let command = string(r, MAX_STRING)?;
    let mode = read_enum(r, &[Sequence, Auto, Redstone], "command block mode")?;
    let flags = r.u8()?;
    Ok(PlayIn::SetCommandBlock(Box::new(CommandBlockUpdate {
        pos,
        command,
        mode,
        track_output: flags & 1 != 0,
        conditional: flags & 2 != 0,
        automatic: flags & 4 != 0,
    })))
}

pub fn read_set_command_minecart(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    Ok(PlayIn::SetCommandMinecart { entity_id: r.varint()?, command: string(r, MAX_STRING)?, track_output: r.bool()? })
}

pub fn read_set_structure_block(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    use StructureMode::*;
    use StructureUpdateType::*;
    let pos = read_position(r)?;
    let update_type = read_enum(r, &[UpdateData, SaveArea, LoadArea, ScanArea], "structure update type")?;
    let mode = read_enum(r, &[Save, Load, Corner, Data], "structure mode")?;
    let name = string(r, MAX_STRING)?;
    let offset = [r.i8()?, r.i8()?, r.i8()?].map(|v| v.clamp(-48, 48));
    let size = [r.i8()?, r.i8()?, r.i8()?].map(|v| v.clamp(0, 48));
    let mirror = read_enum(r, &[Mirror::None, Mirror::LeftRight, Mirror::FrontBack], "mirror")?;
    // Rotation ids wrap around, as in vanilla.
    let rotation = [Rotation::None, Rotation::Clockwise90, Rotation::Clockwise180, Rotation::CounterClockwise90]
        [r.varint()?.rem_euclid(4) as usize];
    let metadata = string(r, MAX_STRUCTURE_METADATA)?;
    let integrity = r.f32()?.clamp(0.0, 1.0);
    let seed = r.varlong()?;
    let flags = r.u8()?;
    Ok(PlayIn::SetStructureBlock(Box::new(StructureBlockUpdate {
        pos,
        update_type,
        mode,
        name,
        offset,
        size,
        mirror,
        rotation,
        metadata,
        integrity,
        seed,
        ignore_entities: flags & 1 != 0,
        show_air: flags & 2 != 0,
        show_bounding_box: flags & 4 != 0,
        strict: flags & 8 != 0,
    })))
}

pub fn read_set_beacon(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    Ok(PlayIn::SetBeacon { primary: optional_varint(r)?, secondary: optional_varint(r)? })
}

pub fn read_edit_book(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    let slot = r.varint()?;
    let count = r.len()?;
    if count > MAX_BOOK_PAGES {
        return Err(DecodeError::Invalid("too many book pages"));
    }
    let pages = (0..count).map(|_| string(r, MAX_BOOK_PAGE)).collect::<Result<_, _>>()?;
    let title = if r.bool()? { Some(string(r, MAX_BOOK_TITLE)?) } else { None };
    Ok(PlayIn::EditBook { slot, pages, title })
}

pub fn read_move_vehicle(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    Ok(PlayIn::MoveVehicle { pos: [r.f64()?, r.f64()?, r.f64()?], rot: [r.f32()?, r.f32()?], on_ground: r.bool()? })
}

pub fn read_select_bundle_item(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    let slot = r.varint()?;
    let index = r.varint()?;
    if index < -1 {
        return Err(DecodeError::Invalid("bundle item index"));
    }
    Ok(PlayIn::SelectBundleItem { slot, index })
}

pub fn read_set_jigsaw_block(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    use super::common::read_identifier;
    Ok(PlayIn::SetJigsawBlock(Box::new(JigsawBlockUpdate {
        pos: read_position(r)?,
        name: read_identifier(r)?,
        target: read_identifier(r)?,
        pool: read_identifier(r)?,
        final_state: string(r, MAX_STRING)?,
        rollable: r.string(MAX_STRING)? == "rollable",
        selection_priority: r.varint()?,
        placement_priority: r.varint()?,
    })))
}

pub fn read_seen_advancements(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    let opened = read_enum(r, &[true, false], "advancements action")?;
    let tab = if opened { Some(super::common::read_identifier(r)?) } else { None };
    Ok(PlayIn::SeenAdvancements { tab })
}

pub fn read_recipe_book_change_settings(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    use RecipeBookType::*;
    Ok(PlayIn::RecipeBookChangeSettings {
        book: read_enum(r, &[Crafting, Furnace, BlastFurnace, Smoker], "recipe book type")?,
        open: r.bool()?,
        filtering: r.bool()?,
    })
}

pub fn read_set_game_rules(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    let count = r.len()?;
    // Each entry takes at least two bytes; this bounds the allocation by the packet size.
    if count > r.remaining() / 2 {
        return Err(DecodeError::Eof);
    }
    let rules = (0..count)
        .map(|_| Ok((super::common::read_identifier(r)?, string(r, MAX_STRING)?)))
        .collect::<Result<_, DecodeError>>()?;
    Ok(PlayIn::SetGameRules { rules })
}

/// `spectator_action`: an optional entity id, sent as id + 1 (0 for none).
pub fn read_spectator_action(r: &mut Reader) -> Result<PlayIn, DecodeError> {
    let v = r.varint()?;
    Ok(PlayIn::SpectatorAction { entity_id: (v != 0).then(|| v - 1) })
}
