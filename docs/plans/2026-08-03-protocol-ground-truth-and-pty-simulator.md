# Protocol Ground Truth + PTY Simulator Implementation Plan

> **For Claude:** ✅ **COMPLETE — do not execute.** All nine tasks shipped. Kept
> for the reasoning and for the caveats in "Critical context" below, which still
> hold. What the plan got wrong or left out is recorded in *Outcome* immediately
> below; read that before trusting any task's stated expectations.

## Outcome

Everything the plan set out to do landed, and two things it did not anticipate
came out of it:

- **Task 5 found a real bug, which is what it was written to do.** Every
  `offset_from_start` constant was five bytes too high — `INPUT_TRIM` landed
  mid-float, `CAB_BYPASS` on a list tag. Corrected against the capture's own
  annotations, per the plan's instruction not to fit the test to the code. The
  plan feared this would mean "M3's write path would corrupt the pedal"; it did
  not. Those five constants back read-only accessors. Every `offset_from_end`
  constant — which is what the write path actually patches through — validated
  unchanged on the first run.
- **The source state dump is a splice.** Its declared size is stale by exactly
  the six bytes of the two fields its author marks "added in 1.2 firmware", so
  header and body come from different firmware versions. It cannot validate
  framing, only offsets. The simulator rebuilds the header for this reason, and
  a test fails the day the capture starts validating on its own.

Two corrections to the plan's own text:

- Task 3 shipped `skip_value` with unbounded recursion; a 20,000-level nested
  input aborted the process with `SIGABRT`. Fixed in `146b8eb` with
  `MAX_LIST_DEPTH`. The plan specified the function without a depth bound.
- Task 6 called for `openpty` + `ptsname`. `openpty` alone is insufficient —
  only `posix_openpt`/`grantpt`/`unlockpt`/`ptsname` yields a device *path*, and
  the path is the point: it is what makes `TtyTransport::open` run the same code
  it will run against `/dev/tonex`.

The plan's closing statement about what remains unverified is unchanged and
still accurate. See `docs/plans/README.md` for the current open-questions list.

**Goal:** Replace the codec's remaining assumptions with real captured hardware data, then build a PTY-backed pedal simulator so `pinex-device` can be written and tested end-to-end on a Mac with no pedal attached.

**Architecture:** Two phases. Phase 1 lands real captured frames from `vit3k/tonex_controller`'s `protocol.md` as fixtures and uses them to settle the `0x80` tag-width question and validate the state field offsets. Phase 2 adds a `Transport` trait to `pinex-device`, a real-tty implementation, and a simulator that speaks the protocol over a `openpty()` pair — so the tty code path is exercised for real, with the handshake anchored to genuine captured bytes.

**Tech Stack:** Rust 2021, existing workspace. Phase 2 adds `nix` (termios + `openpty`) to `pinex-device` only. `pinex-proto` stays pure — its only dependency remains `crc`.

**Critical context — what is and isn't proven:**

- The Hello response in `protocol.md` is **real captured hardware data**, complete with CRC. Its CRC validates against our `CRC_16_IBM_SDLC` implementation and its declared size (43) matches its body length exactly. This is ground truth.
- The state-changed dump in `protocol.md` is real but **has no framing and no CRC** — it is printed "without framing" with prose annotations interleaved. It validates *field offsets*, not framing.
- A simulator encodes our own assumptions. Tests against it prove **plumbing**, not protocol correctness. Every simulator response must be anchored to captured bytes where captured bytes exist, and comments must say so. See @superpowers:testing-anti-patterns.

---

## Phase 1 — Protocol ground truth

### Task 1: Land the captured Hello response as a fixture

**Files:**
- Create: `crates/pinex-proto/tests/fixtures/hello_response.bin` (already in the working tree — see Step 1)
- Modify: `crates/pinex-proto/tests/fixtures/README.md`
- Test: `crates/pinex-proto/tests/fixtures.rs`

**Step 1: Establish RED by moving the fixture aside**

The fixture is already sitting in the working tree from the investigation. Move it out so the new test genuinely fails first:

```bash
mv crates/pinex-proto/tests/fixtures/hello_response.bin /tmp/hello_response.bin
```

**Step 2: Write the failing test**

Append to `crates/pinex-proto/tests/fixtures.rs`:

```rust
/// The Hello response captured from real hardware, transcribed from
/// `vit3k/tonex_controller`'s `protocol.md`. Unlike the synthesized request
/// frames, this is a genuine device reply: its CRC was computed by the pedal.
#[test]
fn captured_hello_response_is_internally_consistent() {
    let bytes = fs::read(fixture_dir().join("hello_response.bin"))
        .expect("hello_response.bin missing");

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
```

Add `MessageType` to the imports at the top of the file:

```rust
use pinex_proto::message::{parse_header_unvalidated, MessageType};
```

**Step 3: Run the test to verify it fails**

Run: `cargo test -p pinex-proto --test fixtures captured_hello_response -- --exact`
Expected: FAIL with `hello_response.bin missing`

**Step 4: Restore the fixture**

```bash
mv /tmp/hello_response.bin crates/pinex-proto/tests/fixtures/hello_response.bin
```

**Step 5: Run the test to verify it passes**

Run: `cargo test -p pinex-proto --test fixtures 2>&1 | tail -5`
Expected: PASS, 2 tests

**Step 6: Record provenance in the fixture README**

In `crates/pinex-proto/tests/fixtures/README.md`, replace the `## What to capture first` section's item 3 and add a provenance section. The README currently claims there are no captures; that is now false.

Add before `## What to capture first`:

```markdown
## Provenance

| File | Source | Firmware | Validates |
|---|---|---|---|
| `hello_response.bin` | Transcribed from [`vit3k/tonex_controller` `protocol.md`](https://github.com/vit3k/tonex_controller/blob/main/protocol.md), "Example response" under *Hello* | 1.1.3 (from the frame itself) | CRC against real hardware; strict header size check on a real response |

`hello_response.bin` was **not** captured by us from a pedal. It is a
transcription of a published capture, byte-for-byte including its CRC. The CRC
validating is meaningful precisely because we did not compute it — the pedal
did.
```

Change item 3 of "What to capture first" from "A Hello response, for the firmware version format" to:

```markdown
3. ~~A Hello response~~ — **done**, see Provenance above. A capture from *our*
   pedal is still worth taking, to confirm the firmware-version format has not
   changed since 1.1.3.
```

**Step 7: Update the harness module doc**

In `crates/pinex-proto/tests/fixtures.rs`, replace the module doc comment's first paragraph:

```rust
//! Regression corpus of real captured frames.
//!
//! Drop a `.bin` of raw bytes read from the tty into `tests/fixtures/` and it
//! gets decoded here — no registration step. See that directory's README for
//! provenance of what is already present.
//!
//! A green run means "nothing contradicted us". For the synthesized request
//! frames that is weak; for `hello_response.bin`, whose CRC was computed by a
//! real pedal, it is strong.
```

**Step 8: Verify and commit**

Run: `cargo test --workspace 2>&1 | grep -E "test result|FAILED"`
Expected: all pass

```bash
git add crates/pinex-proto/tests/
git commit -m "Land captured Hello response as the first real fixture"
```

---

### Task 2: Settle the 0x80 tag-width question

**Files:**
- Modify: `crates/pinex-proto/src/value.rs` (the `tag_width` doc comment)
- Modify: `crates/pinex-proto/src/message.rs` (`request_frames_size_field_discrepancy`)

**Context — the evidence, so the rewrite is not taken on faith.** `protocol.md`'s prose says `0x80` is a 2-byte little-endian integer. Its own captured examples contradict that prose in three independent places:

1. **List element counts.** `0xB9 0x03 0x80 0xFF 0x3F 0x00` is declared as a 3-element collection (`0xB9` is followed by the element count). Width 1 yields three elements — `0x80 0xFF`=255, `0x3F`=63, `0x00`=0, an RGB color. Width 2 yields two. Same for `0xB9 0x03 0x00 0x80 0xFF 0x00`.
2. **The document's own indentation** in the state-changed dump puts `80 97` and `02` on separate lines as separate elements — the width-1 grouping.
3. **The Hello response parses exactly.** Header `b9 03 | 02 | 2b | 0b` then 43 body bytes, matching the declared size of `0x2b`. Verified in Task 1.

This also explains `protocol.md`'s own unanswered note, *"unclear why 0xFF is 'escaped' using a 0x80 prefix"*: bare literals only span `0x00–0x7F`, so a channel value of `0x3F` fits inline but `0xFF` needs the `0x80` tag. It is not escaping.

**The request off-by-one is separately explained.** Requests carry one extra structural byte after the `0x80` field that responses do not (`01` for Hello, `03` for RequestState/RequestPreset). Under width 1, requests are `size + 1`; responses are exactly `size`. Width 2 masked this by absorbing that byte. We only ever parse responses, so `parse_header` is correct as written.

**Step 1: Rewrite the failing test to assert the resolution**

In `crates/pinex-proto/src/message.rs`, replace `request_frames_size_field_discrepancy` entirely:

```rust
    /// Requests carry one structural byte after the `0x80` field that responses
    /// do not, so a request's body is `size + 1`. Responses are exactly `size` —
    /// see `tests/fixtures.rs::captured_hello_response_is_internally_consistent`,
    /// which asserts that against real hardware bytes.
    ///
    /// This asymmetry is why the requests once looked like evidence for a 2-byte
    /// `0x80` tag: that reading absorbed the extra byte. See value::tag_width.
    #[test]
    fn requests_carry_one_extra_header_byte_that_responses_do_not() {
        let preset_payload =
            decode_frame(&request_preset(0, PresetDetail::Summary).unwrap()).unwrap();

        for payload in [HELLO_PAYLOAD, REQUEST_STATE_PAYLOAD, &preset_payload] {
            let header = parse_header_unvalidated(payload).unwrap();
            let remaining = payload.len() - header.body_offset;
            assert_eq!(
                remaining,
                header.size as usize + 1,
                "request should carry exactly one extra byte: {payload:02x?}"
            );
        }
    }
```

**Step 2: Run it**

Run: `cargo test -p pinex-proto requests_carry_one_extra -- --exact`
Expected: PASS (the assertion is unchanged in substance; only its meaning is now documented)

**Step 3: Rewrite the `tag_width` doc comment**

In `crates/pinex-proto/src/value.rs`, replace the entire doc comment above `pub const fn tag_width` with:

```rust
/// How many bytes follow the `0x80` tag: **one**.
///
/// `protocol.md` says `0x80`, `0x81` and `0x82` are all u16 little-endian. Its
/// prose is wrong, and its own captured examples are the proof — a `0xB9 0x03`
/// collection declares three elements, and `0xB9 0x03 0x80 0xFF 0x3F 0x00`
/// only contains three under the 1-byte reading (`0x80 0xFF`=255, `0x3F`=63,
/// `0x00`=0 — an RGB color). Both shipping implementations read it as 1 byte.
///
/// This also answers the question `protocol.md` leaves open — why `0xFF` appears
/// "escaped" with an `0x80` prefix in colors. It is not escaping: bare literals
/// only reach `0x7F`, so `0x3F` fits inline and `0xFF` does not.
///
/// Confirmed against real hardware bytes: the captured Hello response satisfies
/// the strict `remaining == size` check exactly. See
/// `tests/fixtures.rs::captured_hello_response_is_internally_consistent`.
///
/// (Requests are `size + 1` because they carry one extra structural byte that
/// responses lack — not a width problem. See
/// `message::tests::requests_carry_one_extra_header_byte_that_responses_do_not`.)
```

**Step 4: Update the module table at the top of `value.rs`**

Change the `0x80` row from `integer — see [`tag_width`] for the width question` to:

```rust
//! | `0x80`      | integer, 1 byte follows (values `0x80-0xFF`) |
```

**Step 5: Update the fixture harness diagnostic**

In `crates/pinex-proto/tests/fixtures.rs`, the panic message tells the reader an off-by-one means the tag width is wrong. That is no longer the leading hypothesis. Replace:

```rust
                         If the size is off by exactly one, read \
                         pinex_proto::value::tag_width."
```

with:

```rust
                         If the size is off by exactly one, this may be a \
                         request rather than a response — see \
                         message::tests::requests_carry_one_extra_header_byte_that_responses_do_not."
```

**Step 6: Verify and commit**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
Expected: all clean

```bash
git add crates/pinex-proto/src/value.rs crates/pinex-proto/src/message.rs crates/pinex-proto/tests/fixtures.rs
git commit -m "Settle the 0x80 tag width from captured evidence"
```

---

### Task 3: Add a generic value walker

**Why:** Extracting the firmware version means walking to the 4th element of a list whose earlier elements are of mixed types. Every subsequent parser needs this. Without it, parsing is offset arithmetic that breaks the moment a field moves.

**Files:**
- Modify: `crates/pinex-proto/src/value.rs`

**Step 1: Write the failing test**

Add to `value.rs`'s `mod tests`:

```rust
    #[test]
    fn skip_value_advances_past_each_encoding() {
        // literal, 0x80+1, 0x81+2, 0x82+2, f32, and a nested list
        let buf = [
            0x05, // literal
            0x80, 0xc7, // 1-byte int
            0x81, 0xd1, 0x01, // 2-byte int
            0x82, 0x02, 0x00, // 2-byte int
            0x88, 0x00, 0x00, 0x70, 0x41, // f32
            0xb9, 0x03, 0x01, 0x01, 0x03, // 3-element list
        ];

        let mut i = 0;
        for expected in [1, 3, 6, 9, 14, 19] {
            skip_value(&buf, &mut i).unwrap();
            assert_eq!(i, expected);
        }
        assert_eq!(i, buf.len());
    }

    #[test]
    fn skip_value_errors_rather_than_panicking_on_truncation() {
        let mut i = 0;
        assert!(skip_value(&[0x88, 0x00], &mut i).is_err());
    }
```

**Step 2: Run to verify failure**

Run: `cargo test -p pinex-proto skip_value`
Expected: FAIL — `cannot find function skip_value`

**Step 3: Implement**

Add to `value.rs`:

```rust
/// Advance `*index` past one complete value of any type, descending into lists.
///
/// This is how to reach the Nth element of a heterogeneous list without
/// hardcoding byte offsets.
pub fn skip_value(buf: &[u8], index: &mut usize) -> Result<(), ValueError> {
    let tag = *buf.get(*index).ok_or(ValueError::Truncated {
        offset: *index,
        need: 1,
    })?;

    match tag {
        0x80..=0x82 => {
            read_int(buf, index)?;
        }
        0x88 => {
            read_f32(buf, index)?;
        }
        0xB9 | 0xBA | 0xBC => {
            let (_, count) = read_list_header(buf, index)?;
            for _ in 0..count {
                skip_value(buf, index)?;
            }
        }
        // Literal small integer: the tag is the value.
        _ => *index += 1,
    }
    Ok(())
}
```

**Step 4: Run to verify pass**

Run: `cargo test -p pinex-proto skip_value`
Expected: PASS, 2 tests

**Step 5: Commit**

```bash
git add crates/pinex-proto/src/value.rs
git commit -m "Add skip_value walker for heterogeneous lists"
```

---

### Task 4: Parse the firmware version from a Hello response

**Files:**
- Modify: `crates/pinex-proto/src/message.rs`
- Test: `crates/pinex-proto/tests/fixtures.rs`

**Context:** The captured Hello response body is a 7-element `0xB9 0x07` list. Element index 3 is `b9 03 01 01 03` — a 3-element list that is the firmware version `1.1.3`, per the capture's own annotation.

**Step 1: Write the failing test**

Add to `crates/pinex-proto/tests/fixtures.rs`:

```rust
/// The captured response's own annotation says "firmware version - 1.1.3".
/// This asserts we recover exactly that from the real bytes.
#[test]
fn firmware_version_parses_from_the_captured_hello_response() {
    let bytes = fs::read(fixture_dir().join("hello_response.bin")).unwrap();
    let mut acc = FrameAccumulator::new();
    let frames = acc.push(&bytes);
    let body = decode_frame(&frames[0]).unwrap();

    assert_eq!(parse_hello(&body).unwrap(), "1.1.3");
}
```

Add `parse_hello` to the imports.

**Step 2: Run to verify failure**

Run: `cargo test -p pinex-proto --test fixtures firmware_version`
Expected: FAIL — unresolved import `parse_hello`

**Step 3: Implement**

Add to `crates/pinex-proto/src/message.rs`:

```rust
/// Index of the firmware-version element within the Hello response body list.
///
/// Derived from the captured response in `tests/fixtures/hello_response.bin`,
/// whose own annotation identifies element 3 as the version.
const HELLO_FIRMWARE_ELEMENT: u16 = 3;

/// Extract the firmware version string from a Hello response body.
///
/// Returns e.g. `"1.1.3"`. Errors rather than guessing if the shape differs —
/// a firmware update changing this layout must be loud, not silent.
pub fn parse_hello(body: &[u8]) -> Result<String, MessageError> {
    let header = parse_header(body)?;
    let mut index = header.body_offset;

    let (_, count) = read_list_header(body, &mut index)?;
    if count <= HELLO_FIRMWARE_ELEMENT {
        return Err(MessageError::UnexpectedShape {
            what: "hello body list too short for a firmware element",
        });
    }

    for _ in 0..HELLO_FIRMWARE_ELEMENT {
        skip_value(body, &mut index)?;
    }

    let (_, parts) = read_list_header(body, &mut index)?;
    let mut out = String::new();
    for i in 0..parts {
        let part = read_int(body, &mut index)?;
        if i > 0 {
            out.push('.');
        }
        out.push_str(&part.to_string());
    }
    Ok(out)
}
```

Add the `UnexpectedShape` variant to `MessageError`:

```rust
    /// Structure did not match what the captured fixtures show.
    UnexpectedShape {
        what: &'static str,
    },
```

and its `Display` arm:

```rust
            Self::UnexpectedShape { what } => write!(f, "unexpected message shape: {what}"),
```

Extend the `use` at the top of `message.rs`:

```rust
use crate::value::{read_int, read_list_header, skip_value, ValueError};
```

**Step 4: Run to verify pass**

Run: `cargo test -p pinex-proto --test fixtures 2>&1 | tail -5`
Expected: PASS, 3 tests

**Step 5: Verify and commit**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`

```bash
git add crates/pinex-proto/src/message.rs crates/pinex-proto/tests/fixtures.rs
git commit -m "Parse firmware version from the captured Hello response"
```

---

### Task 5: Validate state field offsets against the captured state dump

**Files:**
- Create: `crates/pinex-proto/tests/fixtures/bodies/state_changed.body.bin`
- Create: `crates/pinex-proto/tests/fixtures/bodies/README.md`
- Test: `crates/pinex-proto/tests/state_offsets.rs`

**Why this is a separate directory:** the state dump in `protocol.md` is printed *without framing* — no flags, no CRC. It cannot go in `tests/fixtures/` because the frame harness there would reject it. It validates field offsets, which is what M3's write path depends on.

**Step 1: Extract the body bytes**

The dump spans roughly lines 203–450 of `protocol.md` with prose annotations in `[...]` interleaved. Fetch and strip:

```bash
curl -sS -o /tmp/protocol.md https://raw.githubusercontent.com/vit3k/tonex_controller/main/protocol.md
```

Write a throwaway extractor in the scratchpad that takes the fenced block under `### State Changed`, drops `[...]` annotations, keeps only hex byte tokens, and writes the bytes. **Verify the extraction before trusting it:** the first value after the `b9 01`/`b9 0b` list headers must be `88 00 00 70 41`, the annotated `inputTrim` float of 15.0.

**Step 2: Write the failing test**

Create `crates/pinex-proto/tests/state_offsets.rs`:

```rust
//! Validates PedalState's field offsets against a real captured state message.
//!
//! The capture is body bytes only — `protocol.md` prints it without framing, so
//! there is no CRC here. This proves *offsets*, not framing.

use std::fs;
use std::path::Path;

use pinex_proto::message::parse_header_unvalidated;
use pinex_proto::state::{offset_from_start, PedalState};

fn state_body() -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/bodies/state_changed.body.bin");
    fs::read(p).expect("state_changed.body.bin missing")
}

#[test]
fn captured_state_matches_the_documented_offsets() {
    let raw = state_body();
    let header = parse_header_unvalidated(&raw).unwrap();
    let state = PedalState::from_body(raw[header.body_offset..].to_vec()).unwrap();

    // Annotated in the capture as `cabsimBypass 0x00 - off` and
    // `tunningMode 0x01 - thru`.
    assert_eq!(state.raw()[offset_from_start::CAB_BYPASS], 0x00);
    assert_eq!(state.raw()[offset_from_start::TUNING_MODE], 0x01);

    // Both slots must be within range, and the active slot must resolve.
    assert!(state.active_slot().is_ok());
    assert!(state.active_preset().is_ok());
}
```

**Step 3: Run to verify failure**

Run: `cargo test -p pinex-proto --test state_offsets`
Expected: FAIL — `state_changed.body.bin missing`

**Step 4: Land the extracted bytes and a provenance README**

Write the extracted bytes to the fixture path. Create `crates/pinex-proto/tests/fixtures/bodies/README.md`:

```markdown
# Unframed message bodies

Captured message *bodies* — no flags, no CRC. `protocol.md` prints some captures
without framing, so they cannot go in `../` where the frame harness would reject
them.

| File | Source | Validates |
|---|---|---|
| `state_changed.body.bin` | `protocol.md` § State Changed, prose annotations stripped | `state.rs` field offsets |

These are transcriptions, not our own captures. A real capture from our pedal
supersedes them.
```

**Step 5: Run to verify pass**

Run: `cargo test -p pinex-proto --test state_offsets`
Expected: PASS

**If the offsets do not match:** stop and report. That is a real finding — it means `state.rs`'s constants are wrong and M3's write path would corrupt the pedal. Do not adjust the test to fit; adjust the constants only after confirming against the capture's own annotations, and say so in the commit.

**Step 6: Commit**

```bash
git add crates/pinex-proto/tests/
git commit -m "Validate state field offsets against captured state message"
```

---

## Phase 2 — PTY simulator and `pinex-device`

### Task 6: Transport trait and PTY test harness

**Files:**
- Modify: `crates/pinex-device/Cargo.toml`
- Create: `crates/pinex-device/src/transport.rs`
- Modify: `crates/pinex-device/src/lib.rs`

**Step 1: Add dependencies**

In `crates/pinex-device/Cargo.toml`:

```toml
[dependencies]
pinex-proto.workspace = true
nix = { version = "0.29", features = ["term", "fs", "poll"] }
```

Add to the root `Cargo.toml`'s `[workspace.dependencies]`:

```toml
nix = { version = "0.29", features = ["term", "fs", "poll"] }
```

and reference it as `nix.workspace = true` in the crate.

**Step 2: Write the failing test**

Create `crates/pinex-device/src/transport.rs` with the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A PTY pair is a real tty, so this exercises the same open/termios/read
    /// path a pedal would — only the bytes are ours.
    #[test]
    fn tty_transport_reads_bytes_written_to_the_other_end() {
        let (mut host, device_path) = pty_pair().unwrap();
        let mut transport = TtyTransport::open(&device_path).unwrap();

        host.write_all(b"hello").unwrap();
        host.flush().unwrap();

        let mut buf = [0u8; 16];
        let n = transport.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello");
    }
}
```

**Step 3: Run to verify failure**

Run: `cargo test -p pinex-device`
Expected: FAIL to compile — `pty_pair` and `TtyTransport` not found

**Step 4: Implement `Transport`, `TtyTransport`, and `pty_pair`**

Implement in `transport.rs`:
- `pub trait Transport { fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>; fn write_all(&mut self, buf: &[u8]) -> io::Result<()>; }`
- `pub struct TtyTransport` wrapping an `OwnedFd`, with `open(path) -> io::Result<Self>` that sets raw mode via `nix::sys::termios` (`cfmakeraw`, `VMIN=0`, `VTIME=10` for a 1 s inter-byte timeout matching the accumulator's `flush_stale` contract).
- `pub fn pty_pair() -> io::Result<(File, PathBuf)>` behind `#[cfg(test)]` **or** a `testing` feature, using `nix::pty::openpty` and `ptsname`.

Baud rate is deliberately not set — USB CDC-ACM ignores it.

**Step 5: Run to verify pass**

Run: `cargo test -p pinex-device`
Expected: PASS

**Step 6: Commit**

```bash
git add crates/pinex-device Cargo.toml
git commit -m "Add Transport trait and PTY-backed test harness"
```

---

### Task 7: Reader thread and event bus

**Files:**
- Create: `crates/pinex-device/src/reader.rs`
- Modify: `crates/pinex-device/src/lib.rs`

**Step 1: Write the failing test**

The reader owns the transport, feeds bytes to a `FrameAccumulator`, and emits `PedalEvent` on a channel:

```rust
    #[test]
    fn reader_emits_connected_with_firmware_from_a_real_hello_response() {
        let (mut host, device_path) = pty_pair().unwrap();
        let transport = TtyTransport::open(&device_path).unwrap();
        let (events, rx) = std::sync::mpsc::channel();

        let _reader = Reader::spawn(transport, events);

        // Real captured bytes — see pinex-proto/tests/fixtures/hello_response.bin.
        host.write_all(HELLO_RESPONSE).unwrap();
        host.flush().unwrap();

        match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
            PedalEvent::Connected { firmware } => assert_eq!(firmware, "1.1.3"),
            other => panic!("expected Connected, got {other:?}"),
        }
    }
```

`HELLO_RESPONSE` must be `include_bytes!` of the real fixture, not a hand-written literal — @superpowers:testing-anti-patterns. Reference it across crates via a path include from `pinex-proto`'s fixtures directory.

**Step 2–5:** Run (expect FAIL), implement `PedalEvent`, `Command`, and `Reader::spawn`, run (expect PASS), then commit.

`PedalEvent` per the design doc:

```rust
pub enum PedalEvent {
    Connected { firmware: String },
    Disconnected,
    StateChanged(PedalState),
    PresetNames(Vec<String>),
    ParseError { raw: Vec<u8>, reason: String },
}
```

`ParseError` must be emitted, never logged-and-dropped — it is how a firmware change surfaces.

---

### Task 8: Pedal simulator

**Files:**
- Create: `crates/pinex-device/src/sim.rs` (behind a `sim` feature)

Responds to `Hello` with the captured response verbatim, and to `RequestState` with the captured state body re-framed. Tracks slot/preset in memory so writes are observable.

**The honesty constraint, restated because it is the whole risk of this task:** the simulator's `RequestState` reply is *our* framing around real body bytes. A test that passes against it proves the reader thread, accumulator, and event bus work. It does not prove we parse real state responses correctly. Say so in the module doc, and do not let a green simulator test close out anything the fixtures do not independently support.

**Step: Integration test**

Drive a full handshake — `Hello` → `Connected`, `RequestState` → `StateChanged` — over a PTY, asserting the event sequence.

---

### Task 9: Update the plan record

**Files:**
- Modify: `docs/plans/README.md`
- Modify: `docs/plans/2026-08-02-pinex-proto-scaffolding.md`

Mark finding 8 resolved, pointing at the evidence. Remove the "open questions carried forward" entry for the tag width and replace it with whatever Phase 1 actually left open. Mark this plan complete.

---

## Verification

Full suite after every task:

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

**Still not verified when this plan is done, and worth stating plainly:** that we parse *real* state responses correctly (the captured state body is unframed and transcribed), the preset-name marker, the preset-response type code, and anything touching a pedal, Pi, display, or GPIO. Phase 2 makes the first hardware session a transport swap; it does not substitute for it.
