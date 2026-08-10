//! A simulated Tonex ONE on the far end of a PTY.
//!
//! # How real this is
//!
//! Every reply is **bytes captured from an actual pedal** (firmware 1.3.17),
//! replayed verbatim, CRC and all:
//!
//! - `Hello` → the real Hello response.
//! - `RequestState` → the real state response, while the state is unchanged.
//! - `RequestPreset(n)` → the real response for that preset, all twenty of them.
//!
//! One reply is not purely captured: the state echoed *after a write*. Its body
//! is the real state with the same three bytes patched that a real preset change
//! patches, but the frame has to be rebuilt because the length is then ours to
//! declare. That is the only place left where a green test reflects our framing
//! rather than the pedal's, and a unit test below pins the boundary.
//!
//! # What it still cannot tell you
//!
//! **That the pedal accepts our writes.** The simulator recognises our requests
//! because they are ours; it cannot be surprised by them. Only the real device
//! settles that, which is what `examples/browse.rs` is for.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use pinex_proto::message::{self, parse_header_unvalidated, MessageType, PresetDetail};
use pinex_proto::state::{PedalState, Slot, StateError, MAX_PRESETS};
use pinex_proto::{decode_frame, encode_frame, FrameAccumulator};

use crate::transport::pty_pair;

/// Real captured replies.
const HELLO_RESPONSE: &[u8] =
    include_bytes!("../../pinex-proto/tests/fixtures/hw_hello_fw1_3_17.bin");
const STATE_RESPONSE: &[u8] =
    include_bytes!("../../pinex-proto/tests/fixtures/hw_state_response.bin");

/// All twenty preset replies, captured in index order.
const PRESET_RESPONSES: [&[u8]; MAX_PRESETS as usize] = [
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_00.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_01.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_02.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_03.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_04.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_05.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_06.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_07.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_08.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_09.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_10.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_11.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_12.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_13.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_14.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_15.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_16.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_17.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_18.bin"),
    include_bytes!("../../pinex-proto/tests/fixtures/presets/preset_19.bin"),
];

/// A simulated pedal listening on a PTY. Dropping it stops the thread.
pub struct PedalSim {
    device_path: PathBuf,
    state: Arc<Mutex<PedalState>>,
    unanswered: Arc<AtomicUsize>,
    writes: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl PedalSim {
    /// Start a simulator loaded with the captured state.
    pub fn start() -> io::Result<Self> {
        let (master, device_path) = pty_pair()?;
        set_nonblocking(&master)?;

        let state = Arc::new(Mutex::new(captured_state()?));
        let unanswered = Arc::new(AtomicUsize::new(0));
        let writes = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));

        let handle = {
            let (state, unanswered, writes, stop) = (
                Arc::clone(&state),
                Arc::clone(&unanswered),
                Arc::clone(&writes),
                Arc::clone(&stop),
            );
            std::thread::Builder::new()
                .name("pinex-sim".into())
                .spawn(move || run(master, state, unanswered, writes, stop))?
        };

        Ok(Self {
            device_path,
            state,
            unanswered,
            writes,
            stop,
            handle: Some(handle),
        })
    }

    /// The tty path a [`crate::Pedal`] should open.
    pub fn device_path(&self) -> &Path {
        &self.device_path
    }

    /// The preset the simulated pedal is playing.
    pub fn active_preset(&self) -> u8 {
        self.state
            .lock()
            .unwrap()
            .active_preset()
            .unwrap_or(u8::MAX)
    }

    pub fn active_slot(&self) -> Result<Slot, StateError> {
        self.state.lock().unwrap().active_slot()
    }

    /// Requests the simulator deliberately did not answer.
    pub fn unanswered_requests(&self) -> usize {
        self.unanswered.load(Ordering::Relaxed)
    }

    /// State writes accepted.
    pub fn writes_accepted(&self) -> usize {
        self.writes.load(Ordering::Relaxed)
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

/// The state body out of the captured state response.
fn captured_state() -> io::Result<PedalState> {
    let body = decode_body(STATE_RESPONSE).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "state capture will not decode")
    })?;
    let header = parse_header_unvalidated(&body)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    PedalState::from_body(body[header.body_offset..].to_vec())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

fn decode_body(capture: &[u8]) -> Option<Vec<u8>> {
    let mut acc = FrameAccumulator::new();
    let frames = acc.push(capture);
    decode_frame(frames.first()?).ok()
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
    writes: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
) {
    let mut acc = FrameAccumulator::new();
    let mut buf = [0u8; 4096];

    while !stop.load(Ordering::Relaxed) {
        match master.read(&mut buf) {
            Ok(0) => std::thread::sleep(std::time::Duration::from_millis(2)),
            Ok(n) => {
                for frame in acc.push(&buf[..n]) {
                    if let Some(reply) = respond(&frame, &state, &unanswered, &writes) {
                        if write_fully(&mut master, &reply).is_err() {
                            return;
                        }
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
    }
}

/// Write every byte to a non-blocking fd, waiting out a full buffer.
///
/// The master end is non-blocking so the read loop can poll the stop flag, but
/// that makes writes non-blocking too. A preset reply is ~2.2 KB and a PTY
/// buffer is smaller, so `write_all` would hit `WouldBlock` partway and report
/// failure on a perfectly healthy connection. Retrying is the whole fix.
fn write_fully(master: &mut nix::pty::PtyMaster, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        match master.write(bytes) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "pty took no bytes",
                ))
            }
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                // The reader has not drained yet. Give it a moment.
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    master.flush()
}

/// Decide what a real pedal would send back, or nothing.
fn respond(
    frame: &[u8],
    state: &Arc<Mutex<PedalState>>,
    unanswered: &Arc<AtomicUsize>,
    writes: &Arc<AtomicUsize>,
) -> Option<Vec<u8>> {
    let payload = decode_frame(frame).ok()?;

    // Hello and RequestState share a type code of 0x00, so the header cannot
    // tell them apart. Exact comparison against what pinex-proto emits can, and
    // it also means the simulator only recognises traffic we can produce.
    if payload == decode_frame(&message::hello()).ok()? {
        return Some(HELLO_RESPONSE.to_vec());
    }
    if payload == decode_frame(&message::request_state()).ok()? {
        let guard = state.lock().unwrap();
        return Some(state_response(&guard));
    }

    for preset in 0..MAX_PRESETS {
        let request = message::request_preset(preset, PresetDetail::Summary).ok()?;
        if payload == decode_frame(&request).ok()? {
            return Some(PRESET_RESPONSES[preset as usize].to_vec());
        }
    }

    // A state write: adopt the body and echo the result, as the pedal does.
    if let Ok(header) = parse_header_unvalidated(&payload) {
        if header.msg_type == MessageType::StateUpdate {
            if let Ok(new_state) = PedalState::from_body(payload[header.body_offset..].to_vec()) {
                let mut guard = state.lock().unwrap();
                *guard = new_state;
                writes.fetch_add(1, Ordering::Relaxed);
                return Some(state_response(&guard));
            }
        }
    }

    unanswered.fetch_add(1, Ordering::Relaxed);
    None
}

/// A state-update response around the current body.
///
/// While the state still matches the capture this replays the pedal's own frame
/// byte for byte, so the common path is pure hardware evidence. Once a write has
/// changed the state the frame must be rebuilt, using the header shape the real
/// pedal used: type `0x0306`, a `0x80`-tagged size, trailing `0x0b`.
fn state_response(state: &PedalState) -> Vec<u8> {
    if let Some(body) = decode_body(STATE_RESPONSE) {
        if let Ok(header) = parse_header_unvalidated(&body) {
            if &body[header.body_offset..] == state.raw() {
                return STATE_RESPONSE.to_vec();
            }
        }
    }

    let raw = state.raw();
    let mut payload = vec![0xb9, 0x03, 0x81, 0x06, 0x03];
    // Sizes up to 255 take the one-byte 0x80 form the pedal itself used.
    if raw.len() <= u8::MAX as usize {
        payload.push(0x80);
        payload.push(raw.len() as u8);
    } else {
        payload.push(0x82);
        payload.extend_from_slice(&(raw.len() as u16).to_le_bytes());
    }
    payload.push(0x0b);
    payload.extend_from_slice(raw);
    encode_frame(&payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pinex_proto::message::{parse_header, parse_preset_name};

    #[test]
    fn the_captured_state_is_replayed_verbatim_until_something_changes() {
        let state = captured_state().unwrap();
        assert_eq!(
            state_response(&state),
            STATE_RESPONSE,
            "an unmodified state must be answered with the pedal's own bytes"
        );
    }

    #[test]
    fn a_rebuilt_state_response_still_passes_the_strict_header_check() {
        let mut state = captured_state().unwrap();
        state.stage_preset_in_inactive_slot(9).unwrap();

        let framed = state_response(&state);
        assert_ne!(
            framed, STATE_RESPONSE,
            "the state changed, so the frame must too"
        );

        let payload = decode_frame(&framed).expect("our framing must decode");
        let header = parse_header(&payload).expect("declared size must match the body");
        assert_eq!(header.msg_type, MessageType::StateUpdate);
        assert_eq!(&payload[header.body_offset..], state.raw());
    }

    /// Each captured reply must really be the preset it is filed under, or the
    /// simulator would answer with someone else's name and look correct.
    #[test]
    fn every_captured_preset_reply_carries_its_own_index() {
        for preset in 0..MAX_PRESETS {
            let body = decode_body(PRESET_RESPONSES[preset as usize])
                .unwrap_or_else(|| panic!("preset {preset} will not decode"));
            let info = parse_preset_name(&body).unwrap();
            assert_eq!(info.index, preset, "fixture {preset} is mis-filed");
            assert!(!info.name.is_empty(), "preset {preset} has no name");
        }
    }
}
