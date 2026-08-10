//! Validates `PedalState`'s field offsets against a real captured state message.
//!
//! The capture is body bytes only — `protocol.md` prints it without framing, so
//! there is no CRC here. This proves *offsets*, not framing.
//!
//! Every expectation below is taken from an annotation the capture's author
//! wrote next to the byte. Where the capture does not annotate a byte, this
//! file does not assert a meaning for it.

use std::fs;
use std::path::Path;

use pinex_proto::message::parse_header_unvalidated;
use pinex_proto::state::{offset_from_end, offset_from_start, PedalState};

/// The captured message, header included.
fn state_message() -> Vec<u8> {
    let p =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bodies/state_changed.body.bin");
    fs::read(p).expect("state_changed.body.bin missing")
}

fn state_body() -> Vec<u8> {
    let raw = state_message();
    // Deliberately *unvalidated*: this capture's declared size is stale by
    // exactly the six bytes of the two fields its author marks "added in 1.2
    // firmware version". See the README in tests/fixtures/bodies.
    let header = parse_header_unvalidated(&raw).unwrap();
    raw[header.body_offset..].to_vec()
}

#[test]
fn captured_state_matches_the_annotated_offsets_from_the_start() {
    let state = PedalState::from_body(state_body()).unwrap();
    let raw = state.raw();

    // `88 00 00 70 41 [ inputTrim ... 0x00007041 (15.0) ]`
    let trim = f32::from_le_bytes(
        raw[offset_from_start::INPUT_TRIM..offset_from_start::INPUT_TRIM + 4]
            .try_into()
            .unwrap(),
    );
    assert_eq!(trim, 15.0, "inputTrim");

    // `00 [ cabsimBypass 0x00 - off, 0x01 - on ]`
    assert_eq!(raw[offset_from_start::CAB_BYPASS], 0x00, "cabsimBypass");

    // `01 [ tunningMode 0x00 - mute, 0x01 - thru ]`
    assert_eq!(raw[offset_from_start::TUNING_MODE], 0x01, "tunningMode");

    // `ba 14 [ array of colors for each preset ]` — 0xba list of 0x14 = 20,
    // one per preset, which is itself a check that this is the right byte.
    assert_eq!(raw[offset_from_start::COLORS], 0xba, "colors list tag");
    assert_eq!(
        raw[offset_from_start::COLORS + 1],
        20,
        "one colour per preset"
    );
}

#[test]
fn captured_state_matches_the_annotated_offsets_from_the_end() {
    let state = PedalState::from_body(state_body()).unwrap();
    let raw = state.raw();
    let len = raw.len();

    // `bc 06` slot list: `00 [slot A] 00 02 [slot B] 00 05 [slot C] 00`
    assert_eq!(raw[len - offset_from_end::SLOT_A_PRESET], 0x00, "slot A");
    assert_eq!(raw[len - offset_from_end::SLOT_B_PRESET], 0x02, "slot B");
    assert_eq!(raw[len - offset_from_end::SLOT_C_PRESET], 0x05, "slot C");

    // `00 [ active slot 0 - A, 1 - B, 3 - C ]`
    assert_eq!(
        raw[len - offset_from_end::CURRENT_SLOT],
        0x00,
        "active slot"
    );

    // `81 d1 01 [ a4Reference ... 0x01d1 (465 hz) ]` — the offset points at the
    // value bytes, past the 0x81 tag.
    let tuning = u16::from_le_bytes([
        raw[len - offset_from_end::TUNING_REF],
        raw[len - offset_from_end::TUNING_REF + 1],
    ]);
    assert_eq!(tuning, 465, "a4Reference in Hz");

    // `00 [ direct monitoring 0x00 - off, 0x01 - on ]`
    assert_eq!(
        raw[len - offset_from_end::DIRECT_MONITOR],
        0x00,
        "direct monitoring"
    );

    // `00 [ tempo source 00 - GLOBAL, 01 - PRESET ]`
    assert_eq!(
        raw[len - offset_from_end::TEMPO_SOURCE],
        0x00,
        "tempo source"
    );

    // `88 00 00 70 42 [ tempo in BPM ... 0x00007042 (60) ]`
    let bpm = f32::from_le_bytes(
        raw[len - offset_from_end::BPM..len - offset_from_end::BPM + 4]
            .try_into()
            .unwrap(),
    );
    assert_eq!(bpm, 60.0, "tempo in BPM");
}

/// The offsets the write path depends on must resolve on a real message.
///
/// `stage_preset_in_inactive_slot` writes through exactly these three, so if
/// this test passes, a preset change patches the bytes the pedal expects.
#[test]
fn the_write_path_offsets_resolve_on_a_real_state_message() {
    let mut state = PedalState::from_body(state_body()).unwrap();
    let before = state.raw().to_vec();

    assert_eq!(state.active_slot().unwrap(), pinex_proto::state::Slot::A);
    assert_eq!(state.active_preset().unwrap(), 0);

    let touched = state.stage_preset_in_inactive_slot(7).unwrap();

    assert_eq!(
        pinex_proto::state::diff_offsets(&before, state.raw()),
        touched,
        "a preset change must touch only the offsets it reports"
    );
    assert_eq!(state.active_slot().unwrap(), pinex_proto::state::Slot::B);
    assert_eq!(state.active_preset().unwrap(), 7);
    assert_eq!(state.direct_monitoring(), 1);
    assert_eq!(state.len(), before.len(), "writes must not resize the body");
}
