//! Pure codec for the IK Multimedia Tonex ONE serial protocol.
//!
//! No I/O, no threads, no hardware, no clock. Everything here is testable on a
//! development machine with byte fixtures, which is the entire reason the crate
//! is split out: the protocol is reverse-engineered and firmware-sensitive, so
//! the parsing has to be exercisable without a pedal in the room.
//!
//! # Layers
//!
//! - [`frame`] — HDLC-style stuffing, CRC-16, frame encode/decode
//! - [`accumulator`] — reassembles frames from an unaligned byte stream
//! - [`value`] — tagged-value encoding inside message bodies
//! - [`message`] — request builders and header parsing
//! - [`state`] — pedal state, edited in place rather than re-encoded
//! - [`preset`] — preset-name extraction
//!
//! # Reading is safe; writing needs care
//!
//! There is no "set preset N" command — changing anything means sending the
//! entire state back. [`state`] never re-encodes, and
//! [`state::diff_offsets`] lets a caller prove a write touched only the bytes it
//! meant to. Read [`state`]'s module docs before adding a write path.
//!
//! # Provenance
//!
//! Protocol details come from [`vit3k/tonex_controller`] and
//! [`Builty/TonexOneController`]. Where they disagree with the prose in
//! `protocol.md`, the disagreement is documented at the point it matters — see
//! [`value::tag_width`].
//!
//! [`vit3k/tonex_controller`]: https://github.com/vit3k/tonex_controller
//! [`Builty/TonexOneController`]: https://github.com/Builty/TonexOneController

pub mod accumulator;
pub mod frame;
pub mod message;
pub mod preset;
pub mod state;
pub mod value;

pub use accumulator::FrameAccumulator;
pub use frame::{decode_frame, encode_frame, FrameError};
pub use message::{
    hello, parse_header, request_preset, request_state, write_state, Header, MessageError,
    MessageType, PresetDetail,
};
pub use preset::extract_name;
pub use state::{diff_offsets, PedalState, Slot, StateError, MAX_PRESETS};

/// USB vendor ID of the Tonex ONE.
pub const USB_VID: u16 = 0x1963;
/// USB product ID of the Tonex ONE.
pub const USB_PID: u16 = 0x00D1;
