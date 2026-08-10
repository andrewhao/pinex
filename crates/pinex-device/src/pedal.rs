//! The handle the rest of Pinex binds to.
//!
//! [`Pedal`] is the whole device-facing surface: send a request, receive
//! [`PedalEvent`]s. It does not care whether the tty on the other end is a real
//! Tonex ONE or [`crate::sim::PedalSim`], which is the point — the UI, input and
//! web layers can be written and tested before any hardware exists.
//!
//! Requests are fire-and-forget. The pedal answers on its own schedule and also
//! sends unsolicited state updates when someone turns a knob, so there is no
//! request/response pairing to be had; everything arrives as an event.

use std::io;
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use pinex_proto::message::{self, PresetDetail};
use pinex_proto::state::PedalState;

use crate::reader::{PedalEvent, Reader};
use crate::transport::{Transport, TtyTransport};

/// A connection to a pedal, real or simulated.
pub struct Pedal {
    writer: Box<dyn Transport>,
    events: Receiver<PedalEvent>,
    /// Held so the reader thread lives as long as this handle and is joined on
    /// drop.
    _reader: Reader,
}

impl Pedal {
    /// Open the tty at `path` and start reading from it.
    ///
    /// On the Pi this is `/dev/tonex`; in tests it is a PTY device path.
    pub fn open(path: &Path) -> io::Result<Self> {
        let reading = TtyTransport::open(path)?;
        let writing = reading.try_clone()?;
        Ok(Self::with_transports(reading, writing))
    }

    /// Build from an already-open pair of handles to the same port.
    ///
    /// Two handles rather than one because the reader thread takes ownership of
    /// what it reads; sharing a single one behind a lock would let a slow read
    /// block a footswitch press.
    pub fn with_transports<R: Transport + 'static, W: Transport + 'static>(
        reading: R,
        writing: W,
    ) -> Self {
        let (tx, events) = std::sync::mpsc::channel();
        let reader = Reader::spawn(reading, tx);
        Self {
            writer: Box::new(writing),
            events,
            _reader: reader,
        }
    }

    /// Handshake. The pedal replies with its firmware version.
    pub fn hello(&mut self) -> io::Result<()> {
        self.writer.write_all(&message::hello())
    }

    /// Ask for the complete pedal state.
    pub fn request_state(&mut self) -> io::Result<()> {
        self.writer.write_all(&message::request_state())
    }

    /// Ask for one preset's summary, which is where its name lives.
    pub fn request_preset(&mut self, preset: u8) -> io::Result<()> {
        let framed = message::request_preset(preset, PresetDetail::Summary)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
        self.writer.write_all(&framed)
    }

    /// Send state back to the pedal.
    ///
    /// The body goes out exactly as it came in apart from the patched bytes —
    /// see [`pinex_proto::state`] for why nothing is ever re-encoded.
    pub fn write_state(&mut self, state: &PedalState) -> io::Result<()> {
        self.writer.write_all(&message::write_state(state))
    }

    /// Wait for the next event.
    pub fn next_event(&self, timeout: Duration) -> Result<PedalEvent, RecvTimeoutError> {
        self.events.recv_timeout(timeout)
    }

    /// The event stream, for callers that want to select or iterate on it.
    pub fn events(&self) -> &Receiver<PedalEvent> {
        &self.events
    }
}
