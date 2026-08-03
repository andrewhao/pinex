//! Regression corpus of real captured frames.
//!
//! Drop a `.bin` of raw bytes read from the tty into `tests/fixtures/` and it
//! gets decoded here — no registration step. See that directory's README for
//! provenance of what is already present.
//!
//! A green run means "nothing contradicted us". For the synthesized request
//! frames that is weak; for `hello_response.bin`, whose CRC was computed by a
//! real pedal, it is strong.

use std::fs;
use std::path::{Path, PathBuf};

use pinex_proto::message::{parse_header_unvalidated, MessageType};
use pinex_proto::{decode_frame, parse_header, FrameAccumulator};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixtures() -> Vec<(String, Vec<u8>)> {
    let dir = fixture_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut out: Vec<(String, Vec<u8>)> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "bin"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let bytes = fs::read(&p).unwrap_or_else(|e| panic!("reading {}: {e}", p.display()));
            (name, bytes)
        })
        .collect();

    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn captured_frames_decode() {
    let fixtures = fixtures();
    if fixtures.is_empty() {
        eprintln!(
            "no captures in {} yet — see the README there",
            fixture_dir().display()
        );
        return;
    }

    for (name, bytes) in fixtures {
        let mut acc = FrameAccumulator::new();
        let frames = acc.push(&bytes);

        assert!(
            !frames.is_empty(),
            "{name}: no complete frame found in {} bytes ({} pending, {} dropped)",
            bytes.len(),
            acc.pending(),
            acc.dropped()
        );

        for (i, frame) in frames.iter().enumerate() {
            let body = decode_frame(frame)
                .unwrap_or_else(|e| panic!("{name} frame {i}: {e}\nraw: {frame:02x?}"));

            // Report the header even when the size check fails: a size mismatch
            // is the expected symptom of the `0x80` tag-width ambiguity, and we
            // want the diagnosis rather than a bare failure.
            match parse_header(&body) {
                Ok(header) => eprintln!("{name} frame {i}: {header:?}"),
                Err(strict) => {
                    let loose = parse_header_unvalidated(&body);
                    panic!(
                        "{name} frame {i}: {strict}\n\
                         unvalidated header: {loose:?}\n\
                         body: {body:02x?}\n\
                         If the size is off by exactly one, this may be a \
                         request rather than a response — see message::tests::\
                         requests_carry_one_extra_header_byte_that_responses_do_not."
                    );
                }
            }
        }
    }
}

/// The Hello response captured from real hardware, transcribed from
/// `vit3k/tonex_controller`'s `protocol.md`. Unlike the synthesized request
/// frames, this is a genuine device reply: its CRC was computed by the pedal.
#[test]
fn captured_hello_response_is_internally_consistent() {
    let bytes =
        fs::read(fixture_dir().join("hello_response.bin")).expect("hello_response.bin missing");

    let mut acc = FrameAccumulator::new();
    let frames = acc.push(&bytes);
    assert_eq!(frames.len(), 1, "expected exactly one frame");

    // decode_frame validates the CRC. That it passes means our CRC-16/IBM-SDLC
    // matches the one the pedal actually computed — not a reimplementation.
    let body = decode_frame(&frames[0]).expect("CRC or framing rejected a real frame");

    // parse_header enforces `remaining == size`. That it passes on a real
    // response is what settles the tag-width question; see value::tag_width.
    let header = parse_header(&body).expect("real response failed the strict size check");

    assert_eq!(header.msg_type, MessageType::Hello);
    assert_eq!(header.size, 43);
    assert_eq!(body.len() - header.body_offset, 43);
}
