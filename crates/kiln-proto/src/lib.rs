//! Minecraft Java Edition wire format: primitive codecs, packet framing, network NBT.

pub mod codec;
pub mod frame;
pub mod nbt;

pub use codec::{DecodeError, Reader, WriteExt};
pub use frame::{FrameCodec, MAX_FRAME_LEN, MAX_UNCOMPRESSED_LEN};
