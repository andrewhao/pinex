//! Reader thread: raw bytes in, [`PedalEvent`]s out.
//!
//! One thread owns the transport and is the only thing that reads it. It feeds
//! every byte to a [`FrameAccumulator`], turns whole frames into events, and
//! publishes them on a channel. Everything else in Pinex — renderer, input, web
//! — is a subscriber and never touches the port.
//!
//! **Parse failures are events, not log lines.** A frame we cannot interpret
//! becomes [`PedalEvent::ParseError`] carrying the raw bytes. That is how a
//! firmware change surfaces: loudly, with evidence, instead of as a pedal that
//! quietly stops responding.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;

use pinex_proto::message::{parse_header, parse_hello, MessageType};
use pinex_proto::state::PedalState;
use pinex_proto::{decode_frame, FrameAccumulator};

use crate::transport::Transport;

/// Something the pedal told us.
#[derive(Debug, Clone, PartialEq)]
pub enum PedalEvent {
    Connected {
        firmware: String,
    },
    Disconnected,
    StateChanged(PedalState),
    PresetNames(Vec<String>),
    /// A frame arrived that we could not interpret. Carries the bytes so the
    /// failure can be diagnosed — and turned into a fixture.
    ParseError {
        raw: Vec<u8>,
        reason: String,
    },
}

/// A running reader thread. Dropping this asks the thread to stop and joins it.
#[derive(Debug)]
pub struct Reader {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Reader {
    /// Take ownership of `transport` and start reading on a background thread.
    pub fn spawn<T: Transport + 'static>(transport: T, events: Sender<PedalEvent>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);

        let handle = std::thread::Builder::new()
            .name("pinex-reader".into())
            .spawn(move || run(transport, events, thread_stop))
            .expect("spawning the reader thread");

        Self {
            stop,
            handle: Some(handle),
        }
    }

    /// Ask the thread to stop and wait for it.
    ///
    /// Takes up to one read timeout (~1 s) to return, because the thread only
    /// notices the flag between reads.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run<T: Transport>(mut transport: T, events: Sender<PedalEvent>, stop: Arc<AtomicBool>) {
    let mut acc = FrameAccumulator::new();
    let mut buf = [0u8; 4096];

    while !stop.load(Ordering::Relaxed) {
        let n = match transport.read(&mut buf) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                let _ = events.send(PedalEvent::Disconnected);
                return;
            }
        };

        if n == 0 {
            // An idle timeout, not end-of-stream. Any half-frame still sitting
            // in the accumulator is now stale — the pedal does not pause
            // mid-frame — so drop it rather than let it corrupt the next one.
            acc.flush_stale();
            continue;
        }

        for frame in acc.push(&buf[..n]) {
            // A closed receiver means every subscriber is gone; nothing left to
            // read for.
            if events.send(interpret(&frame)).is_err() {
                return;
            }
        }
    }
}

/// Turn one complete frame into an event, never failing.
fn interpret(frame: &[u8]) -> PedalEvent {
    let body = match decode_frame(frame) {
        Ok(body) => body,
        Err(e) => return parse_error(frame, e),
    };

    let header = match parse_header(&body) {
        Ok(header) => header,
        Err(e) => return parse_error(&body, e),
    };

    match header.msg_type {
        MessageType::Hello => match parse_hello(&body) {
            Ok(firmware) => PedalEvent::Connected { firmware },
            Err(e) => parse_error(&body, e),
        },
        MessageType::StateUpdate => {
            match PedalState::from_body(body[header.body_offset..].to_vec()) {
                Ok(state) => PedalEvent::StateChanged(state),
                Err(e) => parse_error(&body, e),
            }
        }
        // Preset responses use a type code we have not confirmed against
        // hardware, so anything else is reported rather than guessed at.
        MessageType::Unknown(code) => PedalEvent::ParseError {
            raw: body.to_vec(),
            reason: format!("unrecognised message type {code:#06x}"),
        },
    }
}

fn parse_error(raw: &[u8], err: impl std::fmt::Display) -> PedalEvent {
    PedalEvent::ParseError {
        raw: raw.to_vec(),
        reason: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{pty_pair, TtyTransport};
    use std::io::Write;
    use std::time::Duration;

    /// Real captured bytes, not a hand-written literal — the same file
    /// `pinex-proto`'s fixture tests decode.
    const HELLO_RESPONSE: &[u8] =
        include_bytes!("../../pinex-proto/tests/fixtures/hello_response.bin");

    #[test]
    fn reader_emits_connected_with_firmware_from_a_real_hello_response() {
        let (mut host, device_path) = pty_pair().unwrap();
        let transport = TtyTransport::open(&device_path).unwrap();
        let (events, rx) = std::sync::mpsc::channel();

        let _reader = Reader::spawn(transport, events);

        host.write_all(HELLO_RESPONSE).unwrap();
        host.flush().unwrap();

        match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
            PedalEvent::Connected { firmware } => assert_eq!(firmware, "1.1.3"),
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    /// Reads do not align to frame boundaries, so the accumulator must stitch
    /// a frame back together across reads. Splitting mid-frame is the case that
    /// breaks a naive implementation.
    #[test]
    fn a_frame_split_across_reads_still_arrives_whole() {
        let (mut host, device_path) = pty_pair().unwrap();
        let transport = TtyTransport::open(&device_path).unwrap();
        let (events, rx) = std::sync::mpsc::channel();

        let _reader = Reader::spawn(transport, events);

        let (head, tail) = HELLO_RESPONSE.split_at(HELLO_RESPONSE.len() / 2);
        host.write_all(head).unwrap();
        host.flush().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        host.write_all(tail).unwrap();
        host.flush().unwrap();

        match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
            PedalEvent::Connected { firmware } => assert_eq!(firmware, "1.1.3"),
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    /// A firmware change must surface as a reportable event carrying evidence,
    /// never as silence.
    #[test]
    fn an_undecodable_frame_becomes_a_parse_error_carrying_the_raw_bytes() {
        let (mut host, device_path) = pty_pair().unwrap();
        let transport = TtyTransport::open(&device_path).unwrap();
        let (events, rx) = std::sync::mpsc::channel();

        let _reader = Reader::spawn(transport, events);

        // A well-framed frame whose CRC is wrong: flag, junk, flag.
        host.write_all(&[0x7e, 0x01, 0x02, 0x03, 0x04, 0x7e])
            .unwrap();
        host.flush().unwrap();

        match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
            PedalEvent::ParseError { raw, reason } => {
                assert!(!raw.is_empty(), "the raw bytes must be preserved");
                assert!(!reason.is_empty(), "the reason must be reportable");
            }
            other => panic!("expected ParseError, got {other:?}"),
        }
    }

    #[test]
    fn dropping_the_reader_stops_the_thread() {
        let (_host, device_path) = pty_pair().unwrap();
        let transport = TtyTransport::open(&device_path).unwrap();
        let (events, _rx) = std::sync::mpsc::channel();

        let reader = Reader::spawn(transport, events);
        // Returns only once the thread has been joined; hanging here is the
        // failure this guards against.
        reader.stop();
    }
}
