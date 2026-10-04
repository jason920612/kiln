//! World effect packets (play, clientbound): sounds, particles, level and game events, the
//! world border, and block animations (breaking progress, block events, multi-block updates,
//! sign editing). Layouts follow the 26.3 bytecode.

use super::packet;
use crate::WriteExt;
use bytes::{BufMut, Bytes, BytesMut};
use kiln_data::packets::play::clientbound as ids;

// ---- sounds -------------------------------------------------------------------------------

/// A sound event: an entry of `minecraft:sound_event`, or a sound by resource location (for
/// resource pack sounds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sound<'a> {
    /// Protocol id in `minecraft:sound_event` (`kiln_data::builtin_id`).
    Registered(i32),
    /// `fixed_range`: audible distance in blocks; `None` scales with volume (16 blocks at 1.0).
    Direct { id: &'a str, fixed_range: Option<f32> },
}

impl Sound<'_> {
    /// `Holder<SoundEvent>`: registry id + 1, or 0 and the sound inline.
    fn write(&self, b: &mut BytesMut) {
        match *self {
            Sound::Registered(id) => b.put_varint(id + 1),
            Sound::Direct { id, fixed_range } => {
                b.put_varint(0);
                b.put_string(id);
                b.put_bool(fixed_range.is_some());
                if let Some(r) = fixed_range {
                    b.put_f32(r);
                }
            }
        }
    }
}

/// `SoundSource`: the volume slider a sound belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundSource {
    Master,
    Music,
    Records,
    Weather,
    Blocks,
    Hostile,
    Neutral,
    Players,
    Ambient,
    Voice,
    Ui,
}

/// Plays a sound at a position (sent in 1/8 block steps). `seed` picks among the event's
/// variants.
pub fn sound(sound: &Sound, source: SoundSource, pos: [f64; 3], volume: f32, pitch: f32, seed: i64) -> Bytes {
    let mut b = packet(ids::SOUND);
    sound.write(&mut b);
    b.put_varint(source as i32);
    for c in pos {
        b.put_i32((c * 8.0) as i32);
    }
    b.put_f32(volume);
    b.put_f32(pitch);
    b.put_i64(seed);
    b.freeze()
}

/// Plays a sound that follows an entity.
pub fn sound_entity(sound: &Sound, source: SoundSource, entity_id: i32, volume: f32, pitch: f32, seed: i64) -> Bytes {
    let mut b = packet(ids::SOUND_ENTITY);
    sound.write(&mut b);
    b.put_varint(source as i32);
    b.put_varint(entity_id);
    b.put_f32(volume);
    b.put_f32(pitch);
    b.put_i64(seed);
    b.freeze()
}

/// Stops sounds: all of them, those of one source, one sound id, or one sound id in one source.
pub fn stop_sound(source: Option<SoundSource>, sound: Option<&str>) -> Bytes {
    let mut b = packet(ids::STOP_SOUND);
    b.put_u8(source.is_some() as u8 | (sound.is_some() as u8) << 1);
    if let Some(s) = source {
        b.put_varint(s as i32);
    }
    if let Some(id) = sound {
        b.put_string(id);
    }
    b.freeze()
}

// ---- particles ----------------------------------------------------------------------------

/// Where a vibration particle travels to (`PositionSource`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PositionSource {
    Block([i32; 3]),
    Entity { id: i32, y_offset: f32 },
}

/// A particle type's options, per options class in `net.minecraft.core.particles`.
/// Which particle types take which options:
///
/// | options | particle types |
/// |---|---|
/// | `None` | most (`flame`, `heart`, `cloud`, `crit`, ...) |
/// | `Dust` | `dust` |
/// | `DustColorTransition` | `dust_color_transition` |
/// | `Block` | `block`, `block_marker`, `falling_dust`, `dust_pillar`, `block_crumble` |
/// | `Color` | `entity_effect`, `tinted_leaves`, `flash` |
/// | `Spell` | `effect`, `instant_effect` |
/// | `Power` | `dragon_breath` |
/// | `Vibration` | `vibration` |
/// | `SculkCharge` | `sculk_charge` |
/// | `Shriek` | `shriek` |
/// | `Trail` | `trail` |
/// | `Geyser` | `geyser`, `geyser_plume` |
/// | `GeyserBase` | `geyser_base`, `geyser_poof` |
/// | `Raw` | `item` (an `ItemStackTemplate`: item id, count, component patch) |
///
/// Colors are packed `0xRRGGBB` ints (`Color` is `0xAARRGGBB`, alpha included).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParticleOptions<'a> {
    None,
    /// `scale` is clamped to 0.01..=4.0 by the client.
    Dust { color: i32, scale: f32 },
    DustColorTransition { from: i32, to: i32, scale: f32 },
    /// A block state id.
    Block(i32),
    Color(i32),
    Spell { color: i32, power: f32 },
    Power(f32),
    Vibration { destination: PositionSource, arrival_ticks: i32 },
    SculkCharge { roll: f32 },
    Shriek { delay: i32 },
    Trail { target: [f64; 3], color: i32, duration: i32 },
    Geyser { water_blocks: i32 },
    GeyserBase { water_blocks: i32, burst_impulse_base: f32 },
    /// Pre-encoded options, written as they are: the escape hatch for option types without a
    /// variant here (item particles, or types added by later versions).
    Raw(&'a [u8]),
}

impl ParticleOptions<'_> {
    pub fn write(&self, b: &mut BytesMut) {
        match *self {
            ParticleOptions::None => {}
            ParticleOptions::Dust { color, scale } => {
                b.put_i32(color);
                b.put_f32(scale);
            }
            ParticleOptions::DustColorTransition { from, to, scale } => {
                b.put_i32(from);
                b.put_i32(to);
                b.put_f32(scale);
            }
            ParticleOptions::Block(state) => b.put_varint(state),
            ParticleOptions::Color(argb) => b.put_i32(argb),
            ParticleOptions::Spell { color, power } => {
                b.put_i32(color);
                b.put_f32(power);
            }
            ParticleOptions::Power(power) => b.put_f32(power),
            ParticleOptions::Vibration { destination, arrival_ticks } => {
                match destination {
                    PositionSource::Block([x, y, z]) => {
                        b.put_varint(0);
                        b.put_position(x, y, z);
                    }
                    PositionSource::Entity { id, y_offset } => {
                        b.put_varint(1);
                        b.put_varint(id);
                        b.put_f32(y_offset);
                    }
                }
                b.put_varint(arrival_ticks);
            }
            ParticleOptions::SculkCharge { roll } => b.put_f32(roll),
            ParticleOptions::Shriek { delay } => b.put_varint(delay),
            ParticleOptions::Trail { target, color, duration } => {
                target.iter().for_each(|c| b.put_f64(*c));
                b.put_i32(color);
                b.put_varint(duration);
            }
            ParticleOptions::Geyser { water_blocks } => b.put_i32(water_blocks),
            ParticleOptions::GeyserBase { water_blocks, burst_impulse_base } => {
                b.put_i32(water_blocks);
                b.put_f32(burst_impulse_base);
            }
            ParticleOptions::Raw(bytes) => b.put_slice(bytes),
        }
    }

    /// The encoded options, e.g. for `entity::metadata::Particle`.
    pub fn to_vec(&self) -> Vec<u8> {
        let mut b = BytesMut::new();
        self.write(&mut b);
        b.to_vec()
    }
}

/// A particle: its `minecraft:particle_type` protocol id and options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle<'a> {
    pub kind: i32,
    pub options: ParticleOptions<'a>,
}

/// How the client spreads `count` particles (`ParticleRandomization`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParticleRandomization {
    #[default]
    Default,
    Alternative,
    AlternativeWithSpeed,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelParticles<'a> {
    pub particle: Particle<'a>,
    /// Shown even beyond the normal 32-block limit and the client's "decreased" setting.
    pub override_limiter: bool,
    /// Shown even when the client's particle setting is "minimal".
    pub always_show: bool,
    pub pos: [f64; 3],
    /// Gaussian spread per axis; with `count` 0, the direction of a single particle instead.
    pub offset: [f32; 3],
    pub max_speed: [f32; 3],
    pub count: i32,
    pub randomization: ParticleRandomization,
}

pub fn level_particles(p: &LevelParticles) -> Bytes {
    let mut b = packet(ids::LEVEL_PARTICLES);
    b.put_varint(p.particle.kind);
    p.particle.options.write(&mut b);
    b.put_bool(p.override_limiter);
    b.put_bool(p.always_show);
    p.pos.iter().for_each(|c| b.put_f64(*c));
    p.offset.iter().for_each(|c| b.put_f32(*c));
    p.max_speed.iter().for_each(|c| b.put_f32(*c));
    b.put_varint(p.count);
    b.put_varint(p.randomization as i32);
    b.freeze()
}

// ---- level and game events ----------------------------------------------------------------

/// A `LevelEvent` (sound and/or particles the client derives from an id, e.g. 2001 block
/// break with the state id as `data`). `global` plays it regardless of distance.
pub fn level_event(event: i32, pos: [i32; 3], data: i32, global: bool) -> Bytes {
    let mut b = packet(ids::LEVEL_EVENT);
    b.put_i32(event);
    b.put_position(pos[0], pos[1], pos[2]);
    b.put_i32(data);
    b.put_bool(global);
    b.freeze()
}

/// `ClientboundGameEventPacket.Type` ids for [`super::game_event`].
pub mod game_event {
    pub const NO_RESPAWN_BLOCK_AVAILABLE: u8 = 0;
    pub const START_RAINING: u8 = 1;
    pub const STOP_RAINING: u8 = 2;
    /// Value: the game mode id.
    pub const CHANGE_GAME_MODE: u8 = 3;
    /// Value: 0 roll credits without the poem, 1 with it.
    pub const WIN_GAME: u8 = 4;
    /// Value: [`DEMO_INTRO`] or `DEMO_HINT_1..=4` (101..=104).
    pub const DEMO_EVENT: u8 = 5;
    pub const PLAY_ARROW_HIT_SOUND: u8 = 6;
    /// Value: rain level 0.0..=1.0.
    pub const RAIN_LEVEL_CHANGE: u8 = 7;
    /// Value: thunder level 0.0..=1.0.
    pub const THUNDER_LEVEL_CHANGE: u8 = 8;
    pub const PUFFER_FISH_STING: u8 = 9;
    pub const GUARDIAN_ELDER_EFFECT: u8 = 10;
    /// Value: 1 skips the death screen, 0 shows it.
    pub const IMMEDIATE_RESPAWN: u8 = 11;
    /// Value: 1 limits crafting to unlocked recipes.
    pub const LIMITED_CRAFTING: u8 = 12;
    /// `LEVEL_CHUNKS_LOAD_START`: the client waits for chunks before closing the loading screen.
    pub const LEVEL_CHUNKS_LOAD_START: u8 = 13;
    pub const DEMO_INTRO: f32 = 0.0;
}

// ---- world border -------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldBorder {
    pub center: [f64; 2],
    pub size: f64,
    /// Size the border moves to over `lerp_ticks`; equal to `size` when it is not moving.
    pub target_size: f64,
    /// Remaining ticks of the size change.
    pub lerp_ticks: i64,
    /// Vanilla: 29999984.
    pub absolute_max_size: i32,
    pub warning_blocks: i32,
    /// Ticks.
    pub warning_time: i32,
}

pub fn initialize_border(w: &WorldBorder) -> Bytes {
    let mut b = packet(ids::INITIALIZE_BORDER);
    b.put_f64(w.center[0]);
    b.put_f64(w.center[1]);
    b.put_f64(w.size);
    b.put_f64(w.target_size);
    b.put_varlong(w.lerp_ticks);
    b.put_varint(w.absolute_max_size);
    b.put_varint(w.warning_blocks);
    b.put_varint(w.warning_time);
    b.freeze()
}

pub fn set_border_center(x: f64, z: f64) -> Bytes {
    let mut b = packet(ids::SET_BORDER_CENTER);
    b.put_f64(x);
    b.put_f64(z);
    b.freeze()
}

pub fn set_border_lerp_size(size: f64, target_size: f64, lerp_ticks: i64) -> Bytes {
    let mut b = packet(ids::SET_BORDER_LERP_SIZE);
    b.put_f64(size);
    b.put_f64(target_size);
    b.put_varlong(lerp_ticks);
    b.freeze()
}

pub fn set_border_size(size: f64) -> Bytes {
    let mut b = packet(ids::SET_BORDER_SIZE);
    b.put_f64(size);
    b.freeze()
}

/// Warning time in ticks (`WorldBorder.getWarningTime`).
pub fn set_border_warning_delay(ticks: i32) -> Bytes {
    let mut b = packet(ids::SET_BORDER_WARNING_DELAY);
    b.put_varint(ticks);
    b.freeze()
}

// ---- tick rate ----------------------------------------------------------------------------

/// `ClientboundTickingStatePacket`: the tick rate and whether the game is frozen.
pub fn ticking_state(rate: f32, frozen: bool) -> Bytes {
    let mut b = packet(ids::TICKING_STATE);
    b.put_f32(rate);
    b.put_bool(frozen);
    b.freeze()
}

/// `ClientboundTickingStepPacket`: frozen ticks left to run.
pub fn ticking_step(steps: i32) -> Bytes {
    let mut b = packet(ids::TICKING_STEP);
    b.put_varint(steps);
    b.freeze()
}

// ---- biomes -------------------------------------------------------------------------------

/// `ClientboundChunksBiomesPacket`: each chunk's position and its sections' biome containers
/// (as [`kiln_world`-style] paletted containers, already encoded).
pub fn chunks_biomes(chunks: &[([i32; 2], Vec<u8>)]) -> Bytes {
    let mut b = packet(ids::CHUNKS_BIOMES);
    b.put_varint(chunks.len() as i32);
    for ([x, z], data) in chunks {
        b.put_i64(((*z as i64) << 32) | (*x as u32 as i64));
        b.put_varint(data.len() as i32);
        b.put_slice(data);
    }
    b.freeze()
}

pub fn set_border_warning_distance(blocks: i32) -> Bytes {
    let mut b = packet(ids::SET_BORDER_WARNING_DISTANCE);
    b.put_varint(blocks);
    b.freeze()
}

// ---- blocks -------------------------------------------------------------------------------

/// Block breaking cracks shown for a breaker (any entity id; one crack per breaker): stage
/// 0..=9, or `None` to remove them.
pub fn block_destruction(breaker: i32, pos: [i32; 3], stage: Option<u8>) -> Bytes {
    let mut b = packet(ids::BLOCK_DESTRUCTION);
    b.put_varint(breaker);
    b.put_position(pos[0], pos[1], pos[2]);
    b.put_u8(stage.filter(|s| *s <= 9).unwrap_or(0xff));
    b.freeze()
}

/// A block event (note block notes, piston moves, chest lids, ...). `block` is the id in
/// `minecraft:block`, not a state id.
pub fn block_event(pos: [i32; 3], action: u8, param: u8, block: i32) -> Bytes {
    let mut b = packet(ids::BLOCK_EVENT);
    b.put_position(pos[0], pos[1], pos[2]);
    b.put_u8(action);
    b.put_u8(param);
    b.put_varint(block);
    b.freeze()
}

/// Changes several blocks in one 16x16x16 section: section coordinates, then (position
/// within the section as `[x, y, z]` in 0..16, block state id) per block.
pub fn section_blocks_update(section: [i32; 3], blocks: &[([u8; 3], u32)]) -> Bytes {
    let mut b = packet(ids::SECTION_BLOCKS_UPDATE);
    let [x, y, z] = section.map(|c| c as i64);
    b.put_i64((x & 0x3F_FFFF) << 42 | (z & 0x3F_FFFF) << 20 | (y & 0xF_FFFF));
    b.put_varint(blocks.len() as i32);
    for ([lx, ly, lz], state) in blocks {
        let local = ((*lx as i64 & 15) << 8) | ((*lz as i64 & 15) << 4) | (*ly as i64 & 15);
        b.put_varlong((*state as i64) << 12 | local);
    }
    b.freeze()
}

/// Opens the book in a hand (0 main hand, 1 off hand).
pub fn open_book(hand: i32) -> Bytes {
    let mut b = packet(ids::OPEN_BOOK);
    b.put_varint(hand);
    b.freeze()
}

/// Opens the sign editor for the front or back text.
pub fn open_sign_editor(pos: [i32; 3], front: bool) -> Bytes {
    let mut b = packet(ids::OPEN_SIGN_EDITOR);
    b.put_position(pos[0], pos[1], pos[2]);
    b.put_varint(front as i32);
    b.freeze()
}
