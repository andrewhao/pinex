//! Regression corpus of real captured frames.
//!
//! There are no captures yet — no pedal has been connected. This harness exists
//! so that adding one is a file copy and nothing else: drop a `.bin` of raw bytes
//! read from the tty into `tests/fixtures/` and it gets decoded here.
//!
//! It passes trivially while the directory is empty. That is deliberate, and it
//! is why the assertions below print what they found: a green run here means
//! "nothing contradicted us", not "the protocol is verified".

use std::fs;
use std::path::{Path, PathBuf};

use pinex_proto::message::parse_header_unvalidated;
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
                         If the size is off by exactly one, read \
                         pinex_proto::value::tag_width."
                    );
                }
            }
        }
    }
}
