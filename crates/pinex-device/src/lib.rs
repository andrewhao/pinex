//! Serial transport for the Tonex ONE. **Not yet implemented (M1).**
//!
//! Owns `/dev/tonex` (see `deploy/` for the udev rule) and is the only thing in
//! Pinex that touches the port. A reader thread feeds bytes into
//! [`pinex_proto::FrameAccumulator`] and publishes `PedalEvent`s on a channel;
//! renderer, input and web layers are subscribers.
//!
//! Notes carried over from protocol research:
//!
//! - The pedal is plain CDC-ACM. Open the tty as a file and set raw mode via
//!   `nix` termios — no `serialport` crate, so no `libudev` to cross-compile.
//! - Reads do not align to frame boundaries. Drive
//!   [`pinex_proto::FrameAccumulator::flush_stale`] from a ~1s inter-byte
//!   timeout, matching the reference implementation.
//! - ModemManager will probe a CDC-ACM device and send AT commands at it. Rule
//!   this out first if the handshake misbehaves.

#![allow(unused_imports)]

use pinex_proto as _;
