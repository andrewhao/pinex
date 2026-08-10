//! Assertions against frames captured from a real Tonex ONE.
//!
//! Unlike `state_offsets.rs`, which reads a transcription of someone else's
//! published dump, every byte here came off our own pedal over USB — firmware
//! 1.3.17, captured with `cargo run -p pinex-device --example capture`.
//!
//! These are the strongest tests in the workspace: a failure here means we
//! disagree with hardware, not with a document.

use std::fs;
use std::path::{Path, PathBuf};

use pinex_proto::message::{
    parse_header, parse_header_unvalidated, parse_hello, parse_preset_name, MessageType,
};
use pinex_proto::state::{PedalState, Slot};
use pinex_proto::{decode_frame, FrameAccumulator};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Decode the single frame in a capture down to its message body.
fn body_of(name: &str) -> Vec<u8> {
    let bytes = fs::read(fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut acc = FrameAccumulator::new();
    let frames = acc.push(&bytes);
    assert_eq!(frames.len(), 1, "{name}: expected exactly one frame");
    decode_frame(&frames[0]).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn the_pedal_reports_its_firmware_version() {
    let body = body_of("hw_hello_fw1_3_17.bin");
    assert_eq!(parse_header(&body).unwrap().msg_type, MessageType::Hello);
    assert_eq!(parse_hello(&body).unwrap(), "1.3.17");
}

/// The version parser was written against a 1.1.3 transcription. That it also
/// reads a 1.3.17 reply is the evidence that it walks the structure rather than
/// memorising offsets.
#[test]
fn the_same_parser_reads_both_firmware_generations() {
    assert_eq!(
        parse_hello(&body_of("hello_response.bin")).unwrap(),
        "1.1.3"
    );
    assert_eq!(
        parse_hello(&body_of("hw_hello_fw1_3_17.bin")).unwrap(),
        "1.3.17"
    );
}

#[test]
fn a_real_state_response_parses_and_its_end_relative_fields_are_sane() {
    let body = body_of("hw_state_response.bin");
    let header = parse_header(&body).expect("real state response must pass the strict size check");
    assert_eq!(header.msg_type, MessageType::StateUpdate);
    assert_eq!(header.size, 159);

    let state = PedalState::from_body(body[header.body_offset..].to_vec()).unwrap();

    // Read off the pedal at capture time.
    assert_eq!(state.slot_preset(Slot::A), 15);
    assert_eq!(state.slot_preset(Slot::B), 16);
    assert_eq!(state.slot_preset(Slot::C), 1);
    assert_eq!(state.active_slot().unwrap(), Slot::C);
    assert_eq!(state.active_preset().unwrap(), 1);
    assert_eq!(state.direct_monitoring(), 1);
    assert_eq!(state.tuning_reference_hz(), 440);
    assert_eq!(state.tempo_bpm(), 120.0);
}

#[test]
fn preset_responses_carry_their_index_and_name() {
    for (file, index, name) in [
        ("hw_preset0_response.bin", 0u8, "TF BENSON PREAMP - 1"),
        ("hw_preset15_response.bin", 15, "TF TILT - 1 ADV"),
    ] {
        let body = body_of(file);
        assert_eq!(
            parse_header(&body).unwrap().msg_type,
            MessageType::PresetResponse
        );
        let preset = parse_preset_name(&body).unwrap();
        assert_eq!(preset.index, index, "{file}");
        assert_eq!(preset.name, name, "{file}");
    }
}

/// Why `state.rs` has no start-relative offsets any more.
///
/// The fields near the start of the state live inside a list whose element
/// count differs between firmwares. A constant offset into that region reads a
/// different field depending on which firmware answered — silently, with no
/// error. This test is the evidence; it fails the day someone reintroduces the
/// constants on the assumption the layout is fixed.
#[test]
fn start_relative_offsets_would_shift_between_firmwares() {
    let real = body_of("hw_state_response.bin");
    let real_body = &real[parse_header(&real).unwrap().body_offset..];

    let transcribed = fs::read(fixture("bodies/state_changed.body.bin")).unwrap();
    let transcribed_body = &transcribed[8..]; // header is 8 bytes, size field is stale

    // Both open `b9 01` then an inner list — but of different lengths.
    assert_eq!(&real_body[..3], &[0xb9, 0x01, 0xb9]);
    assert_eq!(&transcribed_body[..3], &[0xb9, 0x01, 0xb9]);
    assert_eq!(real_body[3], 0x0e, "firmware 1.3.17 declares 14 elements");
    assert_eq!(
        transcribed_body[3], 0x0b,
        "firmware 1.1.3 declares 11 elements"
    );

    assert_ne!(
        real_body[3], transcribed_body[3],
        "if these ever agree, start-relative offsets are still not safe — \
         the layout would have to be fixed across ALL firmwares, not just two"
    );
}

/// The write path, exercised against the pedal's own state bytes.
///
/// This is the closest thing to a dry run of an actual preset change that can
/// be done without transmitting: same input bytes the pedal sent, same
/// read-modify-write, same verification.
#[test]
fn a_preset_change_built_from_real_state_touches_only_three_bytes() {
    let body = body_of("hw_state_response.bin");
    let header = parse_header(&body).unwrap();
    let state = PedalState::from_body(body[header.body_offset..].to_vec()).unwrap();

    // The pedal was playing slot C, preset 1.
    assert_eq!(state.active_slot().unwrap(), Slot::C);

    let (frame, touched) = pinex_proto::message::set_preset(&state, 7).unwrap();

    // Slot C stages back into A, so: A's preset, the active-slot byte, and the
    // direct-monitoring byte. Nothing else.
    assert_eq!(touched.len(), 3, "touched offsets: {touched:?}");

    // The frame must be a decodable state message whose body is the patched
    // state verbatim — never re-encoded.
    //
    // Read with parse_header_unvalidated, not parse_header: this is a *request*
    // frame, and requests carry one structural byte that responses do not, so
    // the strict `remaining == size` check does not apply. See
    // message::tests::requests_carry_one_extra_header_byte_that_responses_do_not.
    let payload = decode_frame(&frame).expect("our write must be a valid frame");
    let out_header = parse_header_unvalidated(&payload).unwrap();
    assert_eq!(out_header.msg_type, MessageType::StateUpdate);
    assert_eq!(
        out_header.size as usize,
        state.len(),
        "declared body length"
    );

    // The state body is the tail of the payload, byte for byte.
    let sent_body = &payload[payload.len() - state.len()..];
    assert_eq!(sent_body.len(), state.len(), "writes must not resize state");

    let differences: Vec<usize> = (0..state.len())
        .filter(|&i| state.raw()[i] != sent_body[i])
        .collect();
    for offset in &differences {
        assert!(
            touched.contains(offset),
            "offset {offset} changed but was not intended; intended {touched:?}"
        );
    }
    // The pedal already had direct monitoring on, so that intended write is a
    // no-op here. Two of the three intended bytes actually move.
    assert_eq!(differences.len(), 2, "changed offsets: {differences:?}");
}

/// Every preset the pedal can hold must produce a safe write from real state.
#[test]
fn every_preset_index_produces_a_safe_write_from_real_state() {
    let body = body_of("hw_state_response.bin");
    let header = parse_header(&body).unwrap();
    let state = PedalState::from_body(body[header.body_offset..].to_vec()).unwrap();

    for preset in 0..pinex_proto::state::MAX_PRESETS {
        let (_, touched) = pinex_proto::message::set_preset(&state, preset)
            .unwrap_or_else(|e| panic!("preset {preset}: {e}"));
        assert!(touched.len() <= 3, "preset {preset} touched {touched:?}");
    }
    assert!(pinex_proto::message::set_preset(&state, 20).is_err());
}

/// The name sits in a fixed 33-byte buffer followed by its true length. Trusting
/// the buffer alone would drag the null padding into the string.
#[test]
fn preset_names_are_trimmed_to_the_declared_length_not_the_buffer() {
    let preset = parse_preset_name(&body_of("hw_preset15_response.bin")).unwrap();
    assert_eq!(preset.name.len(), 15);
    assert!(!preset.name.contains('\0'), "padding must not leak in");
}
