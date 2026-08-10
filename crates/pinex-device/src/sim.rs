//! A simulated Tonex ONE on the far end of a PTY.
//!
//! # What this is honest about
//!
//! The simulator replies to `Hello` with **captured hardware bytes, verbatim** —
//! the same `hello_response.bin` the codec's fixture tests decode, CRC and all.
//! That reply is real.
//!
//! Its state reply is **not** equally real. The body bytes are captured, but the
//! framing around them is ours: the source dump was printed without framing, and
//! its declared size is stale (see `tests/fixtures/bodies/README.md`), so the
//! header is rebuilt here. A passing test therefore proves the transport, reader
//! thread, accumulator, event bus and codec agree with each other. **It does not
//! prove a real pedal accepts our requests, nor that we parse real state
//! responses correctly.** Do not let a green simulator run close out anything the
//! fixtures do not independently support.
//!
//! Where there is no captured evidence at all — preset responses, whose message
//! type code is unconfirmed — the simulator stays silent and counts the request
//! rather than inventing a plausible reply. A convincing fake is worse than no
//! reply, because it makes a wrong implementation look finished.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use pinex_proto::message::{self, parse_header_unvalidated};
use pinex_proto::state::{PedalState, Slot, StateError};
use pinex_proto::{decode_frame, encode_frame, FrameAccumulator};

use crate::transport::pty_pair;

/// A genuine captured Hello response, replayed byte for byte.
const HELLO_RESPONSE: &[u8] = include_bytes!("../../pinex-proto/tests/fixtures/hello_response.bin");

/// A captured *state message* — header included, but with a stale size field.
/// Only its body is used; the header is rebuilt. See the module docs.
const STATE_MESSAGE: &[u8] =
    include_bytes!("../../pinex-proto/tests/fixtures/bodies/state_changed.body.bin");

/// Message type of a state update, as it appears in a real response header.
const TYPE_STATE_UPDATE: u16 = 0x0306;
/// The header's third field. Purpose unknown; the capture carries `0x02`.
const RESPONSE_UNKNOWN_FIELD: u8 = 0x02;

/// A simulated pedal listening on a PTY.
///
/// Dropping it stops the thread.
pub struct PedalSim {
    device_path: PathBuf,
    state: Arc<Mutex<PedalState>>,
    unanswered: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl PedalSim {
    /// Start a simulator, loaded with the captured state.
    pub fn start() -> io::Result<Self> {
        let (master, device_path) = pty_pair()?;
        set_nonblocking(&master)?;

        let header = parse_header_unvalidated(STATE_MESSAGE)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let state = PedalState::from_body(STATE_MESSAGE[header.body_offset..].to_vec())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        let state = Arc::new(Mutex::new(state));
        let unanswered = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));

        let handle = {
            let state = Arc::clone(&state);
            let unanswered = Arc::clone(&unanswered);
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("pinex-sim".into())
                .spawn(move || run(master, state, unanswered, stop))?
        };

        Ok(Self {
            device_path,
            state,
            unanswered,
            stop,
            handle: Some(handle),
        })
    }

    /// The tty path a [`crate::Pedal`] should open.
    pub fn device_path(&self) -> &Path {
        &self.device_path
    }

    /// The preset the simulated pedal is currently playing.
    pub fn active_preset(&self) -> u8 {
        self.state
            .lock()
            .unwrap()
            .active_preset()
            .unwrap_or(u8::MAX)
    }

    /// The slot the simulated pedal is currently playing.
    pub fn active_slot(&self) -> Result<Slot, StateError> {
        self.state.lock().unwrap().active_slot()
    }

    /// How many requests arrived that the simulator deliberately did not answer.
    pub fn unanswered_requests(&self) -> usize {
        self.unanswered.load(Ordering::Relaxed)
    }
}

impl Drop for PedalSim {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn set_nonblocking(master: &nix::pty::PtyMaster) -> io::Result<()> {
    use nix::fcntl::{fcntl, FcntlArg, OFlag};
    use std::os::fd::AsRawFd;

    let fd = master.as_raw_fd();
    let flags = OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL)?);
    fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
    Ok(())
}

fn run(
    mut master: nix::pty::PtyMaster,
    state: Arc<Mutex<PedalState>>,
    unanswered: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
) {
    let mut acc = FrameAccumulator::new();
    let mut buf = [0u8; 4096];

    while !stop.load(Ordering::Relaxed) {
        match master.read(&mut buf) {
            Ok(0) => std::thread::sleep(std::time::Duration::from_millis(5)),
            Ok(n) => {
                for frame in acc.push(&buf[..n]) {
                    if let Some(reply) = respond(&frame, &state, &unanswered) {
                        if master.write_all(&reply).is_err() {
                            return;
                        }
                        let _ = master.flush();
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
    }
}

/// Decide what a real pedal would send back, or nothing.
fn respond(
    frame: &[u8],
    state: &Arc<Mutex<PedalState>>,
    unanswered: &Arc<AtomicUsize>,
) -> Option<Vec<u8>> {
    let payload = decode_frame(frame).ok()?;

    // Requests are matched byte-for-byte against what `pinex-proto` emits. The
    // Hello and RequestState requests share a type code of 0x00, so the type
    // field cannot tell them apart; exact comparison can, and it also means the
    // simulator only ever recognises traffic we can actually produce.
    if payload == decode_frame(&message::hello()).ok()? {
        return Some(HELLO_RESPONSE.to_vec());
    }

    if payload == decode_frame(&message::request_state()).ok()? {
        return Some(state_response(&state.lock().unwrap()));
    }

    // A state write: header, then the body verbatim. The pedal adopts it and
    // echoes the result back.
    if let Ok(header) = parse_header_unvalidated(&payload) {
        if matches!(
            header.msg_type,
            pinex_proto::message::MessageType::StateUpdate
        ) {
            if let Ok(new_state) = PedalState::from_body(payload[header.body_offset..].to_vec()) {
                let mut guard = state.lock().unwrap();
                *guard = new_state;
                return Some(state_response(&guard));
            }
        }
    }

    // Anything else — notably preset requests, whose response type code is
    // unconfirmed. Counted, not answered. See the module docs.
    unanswered.fetch_add(1, Ordering::Relaxed);
    None
}

/// Build a state-update response around the current body.
///
/// The header is reconstructed rather than replayed, because the captured dump's
/// own size field is stale. The size written here is the true body length, which
/// is what makes the response pass the codec's strict header check.
fn state_response(state: &PedalState) -> Vec<u8> {
    let body = state.raw();
    let mut payload = Vec::with_capacity(body.len() + 16);
    payload.extend_from_slice(&[0xb9, 0x03]);
    payload.push(0x81);
    payload.extend_from_slice(&TYPE_STATE_UPDATE.to_le_bytes());
    payload.push(0x82);
    payload.extend_from_slice(&(body.len() as u16).to_le_bytes());
    payload.push(RESPONSE_UNKNOWN_FIELD);
    payload.extend_from_slice(body);
    encode_frame(&payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pinex_proto::message::parse_header;

    #[test]
    fn a_state_response_satisfies_the_strict_header_check() {
        let header = parse_header_unvalidated(STATE_MESSAGE).unwrap();
        let state = PedalState::from_body(STATE_MESSAGE[header.body_offset..].to_vec()).unwrap();

        let framed = state_response(&state);
        let payload = decode_frame(&framed).expect("our own framing must decode");
        let header = parse_header(&payload).expect("declared size must match the body");

        assert_eq!(payload.len() - header.body_offset, state.len());
    }

    /// The captured dump's stale size is exactly why the header is rebuilt.
    /// If this ever stops holding, the reconstruction is no longer needed.
    #[test]
    fn the_captured_state_message_is_the_one_that_fails_that_check() {
        assert!(
            parse_header(STATE_MESSAGE).is_err(),
            "if the capture now validates, state_response can replay it verbatim"
        );
    }
}
