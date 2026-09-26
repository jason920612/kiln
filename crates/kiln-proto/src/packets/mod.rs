//! Packet bodies (packet id + data) for the states the server speaks so far.
//! Field layouts follow the 26.3 protocol (minecraft.wiki, checked against the jar).

pub mod entity;

use bytes::{BufMut, Bytes, BytesMut};
use kiln_data::packets as ids;
use crate::nbt::Tag;
use crate::{DecodeError, Reader, WriteExt};
use uuid::Uuid;

pub mod commands;
pub mod common;
pub mod hud;
pub mod login_ext;
pub mod player;
pub mod scoreboard;
pub mod serverbound;
pub mod world_fx;

fn packet(id: i32) -> BytesMut {
    let mut b = BytesMut::with_capacity(64);
    b.put_varint(id);
    b
}

// ---- status -------------------------------------------------------------------------------

pub fn status_response(json: &str) -> Bytes {
    let mut b = packet(ids::status::clientbound::STATUS_RESPONSE);
    b.put_string(json);
    b.freeze()
}

pub fn status_pong(ts: i64) -> Bytes {
    let mut b = packet(ids::status::clientbound::PONG_RESPONSE);
    b.put_i64(ts);
    b.freeze()
}

// ---- login --------------------------------------------------------------------------------

pub fn login_disconnect(reason: &str) -> Bytes {
    let mut b = packet(ids::login::clientbound::LOGIN_DISCONNECT);
    b.put_string(&serde_json::json!({ "text": reason }).to_string());
    b.freeze()
}

pub fn login_compression(threshold: i32) -> Bytes {
    let mut b = packet(ids::login::clientbound::LOGIN_COMPRESSION);
    b.put_varint(threshold);
    b.freeze()
}

/// A game profile property, e.g. `textures`.
pub struct ProfileProperty<'a> {
    pub name: &'a str,
    pub value: &'a str,
    pub signature: Option<&'a str>,
}

pub fn put_game_profile(b: &mut BytesMut, uuid: Uuid, name: &str, properties: &[ProfileProperty]) {
    b.put_uuid(uuid);
    b.put_string(name);
    b.put_varint(properties.len() as i32);
    for p in properties {
        b.put_string(p.name);
        b.put_string(p.value);
        b.put_bool(p.signature.is_some());
        if let Some(sig) = p.signature {
            b.put_string(sig);
        }
    }
}

pub fn login_finished(uuid: Uuid, name: &str, properties: &[ProfileProperty], session: Uuid) -> Bytes {
    let mut b = packet(ids::login::clientbound::LOGIN_FINISHED);
    put_game_profile(&mut b, uuid, name, properties);
    b.put_uuid(session);
    b.freeze()
}

// ---- configuration ------------------------------------------------------------------------

pub fn config_brand(brand: &str) -> Bytes {
    let mut b = packet(ids::configuration::clientbound::CUSTOM_PAYLOAD);
    b.put_string("minecraft:brand");
    b.put_string(brand);
    b.freeze()
}

pub fn select_known_packs(packs: &[(&str, &str, &str)]) -> Bytes {
    let mut b = packet(ids::configuration::clientbound::SELECT_KNOWN_PACKS);
    b.put_varint(packs.len() as i32);
    for (ns, id, version) in packs {
        b.put_string(ns);
        b.put_string(id);
        b.put_string(version);
    }
    b.freeze()
}

pub fn update_enabled_features(flags: &[&str]) -> Bytes {
    let mut b = packet(ids::configuration::clientbound::UPDATE_ENABLED_FEATURES);
    b.put_varint(flags.len() as i32);
    for f in flags {
        b.put_string(f);
    }
    b.freeze()
}

/// Registry Data with entry names only; the client fills the data from `minecraft:core`.
pub fn registry_data(registry: &str, entries: &[&str]) -> Bytes {
    let mut b = packet(ids::configuration::clientbound::REGISTRY_DATA);
    b.put_string(registry);
    b.put_varint(entries.len() as i32);
    for e in entries {
        b.put_string(e);
        b.put_bool(false);
    }
    b.freeze()
}

pub fn update_tags(id: i32, tags: &[(&str, &[(&str, &[i32])])]) -> Bytes {
    let mut b = packet(id);
    b.put_varint(tags.len() as i32);
    for (registry, list) in tags {
        b.put_string(registry);
        b.put_varint(list.len() as i32);
        for (tag, entries) in *list {
            b.put_string(tag);
            b.put_varint(entries.len() as i32);
            for e in *entries {
                b.put_varint(*e);
            }
        }
    }
    b.freeze()
}

pub fn finish_configuration() -> Bytes {
    packet(ids::configuration::clientbound::FINISH_CONFIGURATION).freeze()
}

pub fn config_disconnect(reason: &str) -> Bytes {
    let mut b = packet(ids::configuration::clientbound::DISCONNECT);
    crate::nbt::text(reason).write_network(&mut b);
    b.freeze()
}

// ---- play ---------------------------------------------------------------------------------

pub struct Login<'a> {
    pub entity_id: i32,
    pub dimensions: &'a [&'a str],
    pub max_players: i32,
    pub view_distance: i32,
    pub simulation_distance: i32,
    pub dimension_type: i32,
    pub dimension: &'a str,
    pub game_mode: u8,
    pub is_flat: bool,
    pub sea_level: i32,
    pub online_mode: bool,
}

pub fn play_login(l: &Login) -> Bytes {
    let mut b = packet(ids::play::clientbound::LOGIN);
    b.put_i32(l.entity_id);
    b.put_bool(false); // hardcore
    b.put_varint(l.dimensions.len() as i32);
    for d in l.dimensions {
        b.put_string(d);
    }
    b.put_varint(l.max_players);
    b.put_varint(l.view_distance);
    b.put_varint(l.simulation_distance);
    b.put_bool(false); // reduced debug info
    b.put_bool(true); // show respawn screen
    b.put_bool(false); // limited crafting
    player::put_spawn_info(
        &mut b,
        &player::SpawnInfo {
            dimension_type: l.dimension_type,
            dimension: l.dimension,
            hashed_seed: 0,
            game_mode: l.game_mode,
            previous_game_mode: None,
            is_debug: false,
            is_flat: l.is_flat,
            death_location: None,
            portal_cooldown: 0,
            sea_level: l.sea_level,
        },
    );
    b.put_bool(l.online_mode);
    b.put_bool(false); // enforces secure chat
    b.freeze()
}

pub fn game_event(event: u8, value: f32) -> Bytes {
    let mut b = packet(ids::play::clientbound::GAME_EVENT);
    b.put_u8(event);
    b.put_f32(value);
    b.freeze()
}

pub const GAME_EVENT_START_WAITING_FOR_CHUNKS: u8 = 13;

pub fn player_position(teleport_id: i32, pos: [f64; 3], yaw: f32, pitch: f32) -> Bytes {
    let mut b = packet(ids::play::clientbound::PLAYER_POSITION);
    b.put_varint(teleport_id);
    for v in pos {
        b.put_f64(v);
    }
    for _ in 0..3 {
        b.put_f64(0.0); // velocity
    }
    b.put_f32(yaw);
    b.put_f32(pitch);
    b.put_i32(0); // all absolute
    b.freeze()
}

pub fn set_default_spawn_position(dimension: &str, pos: [i32; 3], yaw: f32, pitch: f32) -> Bytes {
    let mut b = packet(ids::play::clientbound::SET_DEFAULT_SPAWN_POSITION);
    b.put_string(dimension);
    b.put_position(pos[0], pos[1], pos[2]);
    b.put_f32(yaw);
    b.put_f32(pitch);
    b.freeze()
}

pub fn set_chunk_cache_center(x: i32, z: i32) -> Bytes {
    let mut b = packet(ids::play::clientbound::SET_CHUNK_CACHE_CENTER);
    b.put_varint(x);
    b.put_varint(z);
    b.freeze()
}

pub fn chunk_batch_start() -> Bytes {
    packet(ids::play::clientbound::CHUNK_BATCH_START).freeze()
}

pub fn chunk_batch_finished(count: i32) -> Bytes {
    let mut b = packet(ids::play::clientbound::CHUNK_BATCH_FINISHED);
    b.put_varint(count);
    b.freeze()
}

/// `body` is everything after the chunk coordinates (heightmaps, sections, block entities, light).
pub fn level_chunk_with_light(x: i32, z: i32, body: &[u8]) -> Bytes {
    let mut b = BytesMut::with_capacity(16 + body.len());
    b.put_varint(ids::play::clientbound::LEVEL_CHUNK_WITH_LIGHT);
    b.put_i32(x);
    b.put_i32(z);
    b.put_slice(body);
    b.freeze()
}

/// `body` is Light Data (masks and arrays) for the changed sections.
pub fn light_update(x: i32, z: i32, body: &[u8]) -> Bytes {
    let mut b = BytesMut::with_capacity(8 + body.len());
    b.put_varint(ids::play::clientbound::LIGHT_UPDATE);
    b.put_varint(x);
    b.put_varint(z);
    b.put_slice(body);
    b.freeze()
}

pub fn forget_level_chunk(x: i32, z: i32) -> Bytes {
    let mut b = packet(ids::play::clientbound::FORGET_LEVEL_CHUNK);
    // ChunkPos written as a single long: z in the high half, x in the low half.
    b.put_i64(((z as i64) << 32) | (x as u32 as i64));
    b.freeze()
}

pub fn keep_alive(id: i64) -> Bytes {
    let mut b = packet(ids::play::clientbound::KEEP_ALIVE);
    b.put_i64(id);
    b.freeze()
}

pub fn system_chat(text: Tag, overlay: bool) -> Bytes {
    let mut b = packet(ids::play::clientbound::SYSTEM_CHAT);
    text.write_network(&mut b);
    b.put_bool(overlay);
    b.freeze()
}

pub fn play_disconnect_text(reason: Tag) -> Bytes {
    let mut b = packet(ids::play::clientbound::DISCONNECT);
    reason.write_network(&mut b);
    b.freeze()
}

/// Chat with a chat type but no signature: `chat_type` is the id in `minecraft:chat_type`.
pub fn disguised_chat(message: &Tag, chat_type: i32, sender: &Tag, target: Option<&Tag>) -> Bytes {
    let mut b = packet(ids::play::clientbound::DISGUISED_CHAT);
    message.write_network(&mut b);
    b.put_varint(chat_type + 1); // Holder: registry id + 1 (0 would be an inline definition)
    sender.write_network(&mut b);
    b.put_bool(target.is_some());
    if let Some(t) = target {
        t.write_network(&mut b);
    }
    b.freeze()
}

pub fn change_difficulty(difficulty: u8, locked: bool) -> Bytes {
    let mut b = packet(ids::play::clientbound::CHANGE_DIFFICULTY);
    b.put_u8(difficulty);
    b.put_bool(locked);
    b.freeze()
}

/// Set Container Slot with an item stack without data components (`None` = empty).
pub fn container_set_slot(window: i32, state: i32, slot: i16, item: Option<(i32, i32)>) -> Bytes {
    let mut b = packet(ids::play::clientbound::CONTAINER_SET_SLOT);
    b.put_varint(window);
    b.put_varint(state);
    b.put_i16(slot);
    put_plain_item(&mut b, item);
    b.freeze()
}

/// Set Container Content: every slot of a container plus the carried item, as item stacks
/// without data components.
pub fn container_set_content(window: i32, state: i32, items: &[Option<(i32, i32)>], carried: Option<(i32, i32)>) -> Bytes {
    let mut b = packet(ids::play::clientbound::CONTAINER_SET_CONTENT);
    b.put_varint(window);
    b.put_varint(state);
    b.put_varint(items.len() as i32);
    for &item in items {
        put_plain_item(&mut b, item);
    }
    put_plain_item(&mut b, carried);
    b.freeze()
}

/// `ItemStack.OPTIONAL_STREAM_CODEC` for a stack with an empty component patch.
fn put_plain_item(b: &mut BytesMut, item: Option<(i32, i32)>) {
    match item {
        Some((id, count)) if count > 0 => {
            b.put_varint(count);
            b.put_varint(id);
            b.put_varint(0); // components added
            b.put_varint(0); // components removed
        }
        _ => b.put_varint(0),
    }
}

/// Set Held Slot: the selected hotbar slot (0-8).
pub fn set_held_slot(slot: i32) -> Bytes {
    let mut b = packet(ids::play::clientbound::SET_HELD_SLOT);
    b.put_varint(slot);
    b.freeze()
}

/// Block Entity Data: the block entity type (protocol id) and its update tag.
pub fn block_entity_data(pos: [i32; 3], kind: i32, tag: &Tag) -> Bytes {
    let mut b = packet(ids::play::clientbound::BLOCK_ENTITY_DATA);
    b.put_position(pos[0], pos[1], pos[2]);
    b.put_varint(kind);
    tag.write_network(&mut b);
    b.freeze()
}

/// Set Container Slot with a stack already encoded with `ItemStack.OPTIONAL_STREAM_CODEC`
/// (kiln-item's `ItemStack::write_optional`).
pub fn container_set_slot_encoded(window: i32, state: i32, slot: i16, stack: &[u8]) -> Bytes {
    let mut b = packet(ids::play::clientbound::CONTAINER_SET_SLOT);
    b.put_varint(window);
    b.put_varint(state);
    b.put_i16(slot);
    b.put_slice(stack);
    b.freeze()
}

pub fn play_disconnect(reason: &str) -> Bytes {
    let mut b = packet(ids::play::clientbound::DISCONNECT);
    crate::nbt::text(reason).write_network(&mut b);
    b.freeze()
}

// ---- serverbound play ---------------------------------------------------------------------

/// Serverbound play packets, decoded by [`decode_play`].
#[derive(Debug, Clone, PartialEq)]
pub enum PlayIn {
    AcceptTeleport { id: i32 },
    KeepAlive { id: i64 },
    Move { pos: Option<[f64; 3]>, rot: Option<[f32; 2]>, on_ground: bool },
    ChunkBatchReceived { chunks_per_tick: f32 },
    ClientInformation(ClientInfo),
    /// Movement keys; bit 0x20 is sneak (shift), 0x40 sprint.
    PlayerInput { flags: u8 },
    /// `player_command` action (1 start sprinting, 2 stop sprinting, ...).
    PlayerCommand { action: i32 },
    /// A command without its leading `/` (signed commands arrive here too; signatures are ignored).
    ChatCommand { command: String },
    CommandSuggestion { id: i32, text: String },
    Chat { message: String },
    PlayerLoaded,
    PlayerAction { action: i32, pos: [i32; 3], face: u8, sequence: i32 },
    UseItemOn { hand: i32, pos: [i32; 3], face: i32, cursor: [f32; 3], inside: bool, sequence: i32 },
    SetCarriedItem { slot: i16 },
    SetCreativeSlot { slot: i16, item: Option<ItemStack> },
    /// Arm swing (26.3 renamed `swing` to `punch`; it has no fields).
    Punch,
    ClientCommand(serverbound::ClientCommand),
    /// Right click on an entity; `location` is the hit point relative to the entity's position.
    Interact { entity_id: i32, hand: serverbound::Hand, location: [f64; 3], sneaking: bool },
    Attack { entity_id: i32 },
    UseItem { hand: serverbound::Hand, sequence: i32, yaw: f32, pitch: f32 },
    ContainerClose { container_id: i32 },
    ContainerButtonClick { container_id: i32, button_id: i32 },
    /// A crafter slot toggled.
    ContainerSlotStateChanged { slot: i32, container_id: i32, enabled: bool },
    PickItemFromBlock { pos: [i32; 3], include_data: bool },
    PickItemFromEntity { entity_id: i32, include_data: bool },
    SignUpdate { pos: [i32; 3], lines: Box<[String; 4]>, front: bool },
    SetCommandBlock(Box<serverbound::CommandBlockUpdate>),
    SetCommandMinecart { entity_id: i32, command: String, track_output: bool },
    SetStructureBlock(Box<serverbound::StructureBlockUpdate>),
    SetJigsawBlock(Box<serverbound::JigsawBlockUpdate>),
    JigsawGenerate { pos: [i32; 3], levels: i32, keep_jigsaws: bool },
    /// Anvil item name (vanilla rejects names over 50 characters when applying it).
    RenameItem { name: String },
    SelectTrade { offer: i32 },
    /// Effect ids in `minecraft:mob_effect`.
    SetBeacon { primary: Option<i32>, secondary: Option<i32> },
    /// `slot` is a hotbar slot (0..9) or 40 (off hand); `title` signs the book.
    EditBook { slot: i32, pages: Vec<String>, title: Option<String> },
    /// Only the flying flag is read.
    PlayerAbilities { flying: bool },
    ClientTickEnd,
    PaddleBoat { left: bool, right: bool },
    MoveVehicle { pos: [f64; 3], rot: [f32; 2], on_ground: bool },
    /// 0 peaceful ..= 3 hard (ids wrap, as in vanilla).
    ChangeDifficulty { difficulty: u8 },
    LockDifficulty { locked: bool },
    /// Game mode id from the F3+F4 switcher (unknown ids are survival, as in vanilla).
    ChangeGameMode { game_mode: u8 },
    /// Spectator menu teleport.
    TeleportToEntity { target: Uuid },
    /// Spectate an entity (or stop, with `None`).
    SpectatorAction { entity_id: Option<i32> },
    ChatAck { offset: i32 },
    /// `index` -1 deselects.
    SelectBundleItem { slot: i32, index: i32 },
    /// `tab` is the opened advancement tab, `None` when the screen closed.
    SeenAdvancements { tab: Option<String> },
    RecipeBookSeenRecipe { recipe: i32 },
    RecipeBookChangeSettings { book: serverbound::RecipeBookType, open: bool, filtering: bool },
    PlaceRecipe { container_id: i32, recipe: i32, use_max_items: bool },
    BlockEntityTagQuery { transaction: i32, pos: [i32; 3] },
    EntityTagQuery { transaction: i32, entity_id: i32 },
    /// (game rule key, value) pairs from the edit game rules screen.
    SetGameRules { rules: Vec<(String, String)> },
    /// Network debug screen ping; answer with `common::pong_response(time)`.
    PingRequest { time: i64 },
    /// Answer to `common::ping`.
    Pong { id: i32 },
    ResourcePack { id: Uuid, action: common::ResourcePackAction },
    CookieResponse(common::CookieResponse),
    /// A `minecraft:custom` click action from a dialog or chat.
    CustomClickAction { id: String, payload: Option<Tag> },
}

/// An item stack as sent by the client. Component patches are kept as raw, length-delimited
/// entries (the "untrusted" slot codec prefixes each component with its length).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemStack {
    pub item: i32,
    pub count: i32,
    pub added: Vec<(i32, Bytes)>,
    pub removed: Vec<i32>,
}

/// Block position packed as x:26, z:26, y:12.
pub fn read_position(r: &mut Reader) -> Result<[i32; 3], DecodeError> {
    let v = r.i64()?;
    Ok([(v >> 38) as i32, (v << 52 >> 52) as i32, (v << 26 >> 38) as i32])
}

/// `ItemStack.OPTIONAL_UNTRUSTED_STREAM_CODEC`: count, then item id and a delimited component patch.
pub fn read_untrusted_slot(r: &mut Reader) -> Result<Option<ItemStack>, DecodeError> {
    let count = r.varint()?;
    if count <= 0 {
        return Ok(None);
    }
    let item = r.varint()?;
    let adds = r.len()?;
    let removes = r.len()?;
    if adds + removes > 256 {
        return Err(DecodeError::Invalid("too many item components"));
    }
    let mut added = Vec::with_capacity(adds);
    for _ in 0..adds {
        let ty = r.varint()?;
        let len = r.len()?;
        if len > 2 * 1024 * 1024 {
            return Err(DecodeError::Invalid("item component too large"));
        }
        added.push((ty, Bytes::copy_from_slice(r.bytes(len)?)));
    }
    let mut removed = Vec::with_capacity(removes);
    for _ in 0..removes {
        removed.push(r.varint()?);
    }
    Ok(Some(ItemStack { item, count, added, removed }))
}

/// World clock state for Set Time: registry id in `minecraft:world_clock`, time, rate.
pub struct ClockState {
    pub clock: i32,
    pub time: i64,
    pub fraction: f32,
    pub rate: f32,
}

/// 26.x Set Time: world age plus the state of each world clock.
pub fn set_time(game_time: i64, clocks: &[ClockState]) -> Bytes {
    let mut b = packet(ids::play::clientbound::SET_TIME);
    b.put_i64(game_time);
    b.put_varint(clocks.len() as i32);
    for c in clocks {
        b.put_varint(c.clock);
        b.put_varlong(c.time);
        b.put_f32(c.fraction);
        b.put_f32(c.rate);
    }
    b.freeze()
}

pub fn block_update(pos: [i32; 3], state: u16) -> Bytes {
    let mut b = packet(ids::play::clientbound::BLOCK_UPDATE);
    b.put_position(pos[0], pos[1], pos[2]);
    b.put_varint(state as i32);
    b.freeze()
}

pub fn block_changed_ack(sequence: i32) -> Bytes {
    let mut b = packet(ids::play::clientbound::BLOCK_CHANGED_ACK);
    b.put_varint(sequence);
    b.freeze()
}

/// Decodes a serverbound play packet; `Ok(None)` for packets we ignore for now.
pub fn decode_play(id: i32, r: &mut Reader) -> Result<Option<PlayIn>, DecodeError> {
    use ids::play::serverbound as sb;
    let pkt = match id {
        sb::ACCEPT_TELEPORTATION => {
            let id = r.varint()?;
            r.rest(); // echoed position and rotation
            PlayIn::AcceptTeleport { id }
        }
        sb::KEEP_ALIVE => PlayIn::KeepAlive { id: r.i64()? },
        sb::MOVE_PLAYER_POS => {
            let pos = [r.f64()?, r.f64()?, r.f64()?];
            PlayIn::Move { pos: Some(pos), rot: None, on_ground: r.u8()? & 1 != 0 }
        }
        sb::MOVE_PLAYER_POS_ROT => {
            let pos = [r.f64()?, r.f64()?, r.f64()?];
            let rot = [r.f32()?, r.f32()?];
            PlayIn::Move { pos: Some(pos), rot: Some(rot), on_ground: r.u8()? & 1 != 0 }
        }
        sb::MOVE_PLAYER_ROT => {
            let rot = [r.f32()?, r.f32()?];
            PlayIn::Move { pos: None, rot: Some(rot), on_ground: r.u8()? & 1 != 0 }
        }
        sb::MOVE_PLAYER_STATUS_ONLY => PlayIn::Move { pos: None, rot: None, on_ground: r.u8()? & 1 != 0 },
        sb::CHUNK_BATCH_RECEIVED => PlayIn::ChunkBatchReceived { chunks_per_tick: r.f32()? },
        sb::CLIENT_INFORMATION => PlayIn::ClientInformation(read_client_information(r)?),
        sb::PLAYER_INPUT => PlayIn::PlayerInput { flags: r.u8()? },
        sb::PLAYER_COMMAND => {
            let _entity = r.varint()?;
            let action = r.varint()?;
            let _data = r.varint()?;
            PlayIn::PlayerCommand { action }
        }
        sb::CHAT_COMMAND => PlayIn::ChatCommand { command: commands::decode_chat_command(r)? },
        sb::CHAT_COMMAND_SIGNED => PlayIn::ChatCommand { command: commands::decode_chat_command_signed(r)?.command },
        sb::COMMAND_SUGGESTION => {
            let req = commands::decode_command_suggestion(r)?;
            PlayIn::CommandSuggestion { id: req.id, text: req.command }
        }
        sb::CHAT => {
            let message = r.string(256)?.to_owned();
            r.rest(); // timestamp, salt, signature, acknowledgements: unsigned chat for now
            PlayIn::Chat { message }
        }
        sb::PLAYER_LOADED => PlayIn::PlayerLoaded,
        sb::PLAYER_ACTION => PlayIn::PlayerAction {
            action: r.varint()?,
            pos: read_position(r)?,
            face: r.u8()?,
            sequence: r.varint()?,
        },
        sb::USE_ITEM_ON => PlayIn::UseItemOn {
            hand: r.varint()?,
            pos: read_position(r)?,
            face: r.varint()?,
            cursor: [r.f32()?, r.f32()?, r.f32()?],
            inside: r.bool()?,
            sequence: {
                let _world_border_hit = r.bool()?;
                r.varint()?
            },
        },
        sb::SET_CARRIED_ITEM => PlayIn::SetCarriedItem { slot: r.i16()? },
        sb::SET_CREATIVE_MODE_SLOT => PlayIn::SetCreativeSlot { slot: r.i16()?, item: read_untrusted_slot(r)? },
        sb::PUNCH => PlayIn::Punch,
        sb::CLIENT_COMMAND => serverbound::read_client_command(r)?,
        sb::INTERACT => serverbound::read_interact(r)?,
        sb::ATTACK => PlayIn::Attack { entity_id: r.varint()? },
        sb::USE_ITEM => serverbound::read_use_item(r)?,
        sb::CONTAINER_CLOSE => PlayIn::ContainerClose { container_id: r.varint()? },
        sb::CONTAINER_BUTTON_CLICK => PlayIn::ContainerButtonClick { container_id: r.varint()?, button_id: r.varint()? },
        sb::CONTAINER_SLOT_STATE_CHANGED => {
            PlayIn::ContainerSlotStateChanged { slot: r.varint()?, container_id: r.varint()?, enabled: r.bool()? }
        }
        sb::PICK_ITEM_FROM_BLOCK => PlayIn::PickItemFromBlock { pos: read_position(r)?, include_data: r.bool()? },
        sb::PICK_ITEM_FROM_ENTITY => PlayIn::PickItemFromEntity { entity_id: r.varint()?, include_data: r.bool()? },
        sb::SIGN_UPDATE => serverbound::read_sign_update(r)?,
        sb::SET_COMMAND_BLOCK => serverbound::read_set_command_block(r)?,
        sb::SET_COMMAND_MINECART => serverbound::read_set_command_minecart(r)?,
        sb::SET_STRUCTURE_BLOCK => serverbound::read_set_structure_block(r)?,
        sb::SET_JIGSAW_BLOCK => serverbound::read_set_jigsaw_block(r)?,
        sb::JIGSAW_GENERATE => PlayIn::JigsawGenerate { pos: read_position(r)?, levels: r.varint()?, keep_jigsaws: r.bool()? },
        sb::RENAME_ITEM => PlayIn::RenameItem { name: r.string(32767)?.to_owned() },
        sb::SELECT_TRADE => PlayIn::SelectTrade { offer: r.varint()? },
        sb::SET_BEACON => serverbound::read_set_beacon(r)?,
        sb::EDIT_BOOK => serverbound::read_edit_book(r)?,
        sb::PLAYER_ABILITIES => PlayIn::PlayerAbilities { flying: r.u8()? & 0x02 != 0 },
        sb::CLIENT_TICK_END => PlayIn::ClientTickEnd,
        sb::PADDLE_BOAT => PlayIn::PaddleBoat { left: r.bool()?, right: r.bool()? },
        sb::MOVE_VEHICLE => serverbound::read_move_vehicle(r)?,
        sb::CHANGE_DIFFICULTY => PlayIn::ChangeDifficulty { difficulty: r.varint()?.rem_euclid(4) as u8 },
        sb::LOCK_DIFFICULTY => PlayIn::LockDifficulty { locked: r.bool()? },
        sb::CHANGE_GAME_MODE => {
            let id = r.varint()?;
            PlayIn::ChangeGameMode { game_mode: if (0..4).contains(&id) { id as u8 } else { 0 } }
        }
        sb::TELEPORT_TO_ENTITY => PlayIn::TeleportToEntity { target: r.uuid()? },
        sb::SPECTATOR_ACTION => serverbound::read_spectator_action(r)?,
        sb::CHAT_ACK => PlayIn::ChatAck { offset: r.varint()? },
        sb::BUNDLE_ITEM_SELECTED => serverbound::read_select_bundle_item(r)?,
        sb::SEEN_ADVANCEMENTS => serverbound::read_seen_advancements(r)?,
        sb::RECIPE_BOOK_SEEN_RECIPE => PlayIn::RecipeBookSeenRecipe { recipe: r.varint()? },
        sb::RECIPE_BOOK_CHANGE_SETTINGS => serverbound::read_recipe_book_change_settings(r)?,
        sb::PLACE_RECIPE => {
            PlayIn::PlaceRecipe { container_id: r.varint()?, recipe: r.varint()?, use_max_items: r.bool()? }
        }
        sb::BLOCK_ENTITY_TAG_QUERY => PlayIn::BlockEntityTagQuery { transaction: r.varint()?, pos: read_position(r)? },
        sb::ENTITY_TAG_QUERY => PlayIn::EntityTagQuery { transaction: r.varint()?, entity_id: r.varint()? },
        sb::SET_GAME_RULE => serverbound::read_set_game_rules(r)?,
        sb::PING_REQUEST => PlayIn::PingRequest { time: r.i64()? },
        sb::PONG => PlayIn::Pong { id: r.i32()? },
        sb::RESOURCE_PACK => {
            let (id, action) = common::read_resource_pack_response(r)?;
            PlayIn::ResourcePack { id, action }
        }
        sb::COOKIE_RESPONSE => PlayIn::CookieResponse(common::read_cookie_response(r)?),
        sb::CUSTOM_CLICK_ACTION => {
            let (id, payload) = common::read_custom_click_action(r)?;
            PlayIn::CustomClickAction { id, payload }
        }
        _ => {
            r.rest();
            return Ok(None);
        }
    };
    r.finish()?;
    Ok(Some(pkt))
}

/// Client settings that affect what the server sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientInfo {
    pub view_distance: u8,
    /// Displayed skin layers (bit mask; 0x7f is all layers).
    pub skin_parts: u8,
    /// 0 left, 1 right.
    pub main_hand: i32,
}

impl Default for ClientInfo {
    fn default() -> Self {
        Self { view_distance: 8, skin_parts: 0x7f, main_hand: 1 }
    }
}

/// Client Information (configuration and play).
pub fn read_client_information(r: &mut Reader) -> Result<ClientInfo, DecodeError> {
    let _locale = r.string(16)?;
    let view_distance = r.i8()?.max(2) as u8;
    let _chat_mode = r.varint()?;
    let _chat_colors = r.bool()?;
    let skin_parts = r.u8()?;
    let main_hand = r.varint()?;
    let _text_filtering = r.bool()?;
    let _allow_listing = r.bool()?;
    let _particles = r.varint()?;
    Ok(ClientInfo { view_distance, skin_parts, main_hand })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_roundtrip_including_negatives() {
        for pos in [[18357644, 831, -20882616], [-1, -64, -1], [0, 0, 0], [-33554432, -2048, 33554431]] {
            let mut b = BytesMut::new();
            b.put_position(pos[0], pos[1], pos[2]);
            assert_eq!(read_position(&mut Reader::new(&b)).unwrap(), pos);
        }
    }

    #[test]
    fn encoded_slot_matches_plain_slot() {
        assert_eq!(container_set_slot(0, 3, 36, Some((42, 5))), container_set_slot_encoded(0, 3, 36, &[5, 42, 0, 0]));
        assert_eq!(container_set_slot(0, 3, 36, None), container_set_slot_encoded(0, 3, 36, &[0]));
    }

    #[test]
    fn untrusted_slot_skips_delimited_components() {
        let mut b = BytesMut::new();
        b.put_varint(3); // count
        b.put_varint(42); // item
        b.put_varint(1); // one added component
        b.put_varint(1); // one removed component
        b.put_varint(7);
        b.put_varint(2);
        b.put_slice(&[0xAA, 0xBB]);
        b.put_varint(9);
        let mut r = Reader::new(&b);
        let s = read_untrusted_slot(&mut r).unwrap().unwrap();
        r.finish().unwrap();
        assert_eq!((s.item, s.count, s.removed.as_slice()), (42, 3, &[9][..]));
        assert_eq!(&s.added[0].1[..], &[0xAA, 0xBB]);
        assert_eq!(read_untrusted_slot(&mut Reader::new(&[0])).unwrap(), None);
    }
}
