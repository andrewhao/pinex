# Pinex — M0 Scaffolding + `pinex-proto` Implementation Plan

> **For Claude:** ✅ **COMPLETE — do not execute.** Shipped in commit `3a390ff`
> on branch `pinex-proto`. This is kept as the record of *why* the code looks the
> way it does; the research findings below are the durable part and are cited
> from `crates/pinex-proto/` doc comments. Superseded three claims in
> [`2026-08-01-pinex-design.md`](2026-08-01-pinex-design.md).

**Goal:** Ship the Cargo workspace (M0) and `pinex-proto` (M1 step 1) — the pure,
hardware-free Tonex ONE codec that can be fully unit-tested with no pedal and no Pi.

**Architecture:** `pinex-proto` is pure: no I/O, no threads, no hardware, no
`std::fs`. `PedalState` holds the raw response body verbatim and patches
individual bytes in place, so undocumented fields survive by construction. The
five downstream crates are documented stubs so the layering is visible to
`cargo check`.

**Tech Stack:** Rust 2021, Cargo workspace (resolver 2), one dependency — `crc`
(`CRC_16_IBM_SDLC`). No `serialport`, deliberately (avoids `libudev`
cross-compilation pain).

**Verification:** `cargo test --workspace` — 51 unit tests + 1 property test +
fixture harness; `cargo clippy --workspace --all-targets -- -D warnings`;
`cargo fmt --all --check`. All green.

**Follow-on:** M1 step 2 (`pinex-device`) needs the pedal. The top item for the
first hardware session is capturing one `RequestState` response — it settles
finding 8 below, which is currently pinned by a test rather than resolved.

---

## Context

`andrewhao/pinex` contains exactly two files: `PLAN.md` and `.gitignore`. The design work is done and locked; nothing has been built. This plan executes the first slice.

`PLAN.md` was written from `protocol.md` alone. I read the source of **both** reference implementations, which confirmed some guesses, **contradicted two design decisions**, and surfaced a gotcha that would have cost a confusing debugging session. Those findings are below — they change what gets built, so read that section before the build plan.

**Scope of this slice:** the Cargo workspace (M0) and `pinex-proto` (M1 step 1) — the pure, hardware-free protocol crate. This container has Rust 1.94.1 on x86_64 but no Pi, no pedal, no `aarch64` target, and no Docker/`cross`, so `pinex-proto` is the only part that can ship *verified* rather than merely compiling. Everything downstream of it is unchanged from `PLAN.md`.

## Research findings that change the plan

Sources: [`vit3k/tonex_controller`](https://github.com/vit3k/tonex_controller) (`main/hdlc.cpp`, `main/tonex.cpp`) and [`Builty/TonexOneController`](https://github.com/Builty/TonexOneController) (`source/main/usb_tonex_one.c`).

### 1. CRC is confirmed — de-risk it

`hdlc.cpp:33-46` computes: reflected, poly `0x8408` (reversed `0x1021`), init `0xFFFF`, final `~crc` (xorout `0xFFFF`), over the **unstuffed** payload between flags, appended **little-endian** (low byte first), then stuffed. That is exactly `CRC_16_IBM_SDLC`. `PLAN.md`'s guess was right. Demote this from the risk table — it is now a fixture assertion, not an open question.

Stuffing: escape `0x7E` and `0x7D` as `0x7D` followed by `byte ^ 0x20`.

### 2. Never re-serialize state — patch the raw buffer *(supersedes the "central hazard")*

`PLAN.md` describes read → parse → mutate → **re-serialize** → diff byte-for-byte, and calls the corruption risk "the single fact that drives milestone ordering."

Neither reference does this, and the risk is an artifact of the approach. Both keep the **raw byte tail** of the state response and overwrite **one byte in place**, then prepend a freshly built header (`tonex.cpp:100-136`, `usb_tonex_one.c:443-530`). Unknown fields are preserved *by construction* because they are never decoded or re-encoded.

Adopt this. `PedalState` holds `raw: Vec<u8>` as the source of truth with parsed fields as a read-only *view*. The byte-for-byte diff `PLAN.md` wants becomes a cheap invariant: assert exactly the intended offsets differ. This removes the whole corruption class rather than testing for it.

### 3. State field offsets (two independent sources agree)

Into the state body, after the header. Both implementations agree on the slot offsets.

| From start | Field | | From end (`len - N`) | Field |
|---|---|---|---|---|
| 15 | input trim (f32) | | 4 | BPM |
| 19 | stomp mode (0=A/B, 1=stomp) | | 6 | tempo source |
| 20 | cab bypass | | 7 | **direct monitoring** |
| 21 | tuning mode (0=mute, 1=thru) | | 9 | tuning reference |
| 22 | preset colors start | | 11 | **current slot** |
| | | | 12 | bypass mode |
| | | | 14 | slot C preset |
| | | | 16 | slot B preset |
| | | | 18 | slot A preset |

### 4. USB connection can mute the pedal — the direct-monitor byte

`usb_tonex_one.c:478` sets `StateData[len - 7] = 1` with the comment *"make sure direct monitoring is on so sound not muted from USB connection."* Merely being connected over USB can silence output. This is the kind of thing that reads as broken hardware.

Consequence: M3's first write touches **two** bytes (preset index + direct monitor), not one. The diff invariant in finding #2 must allow exactly that set — encode it as an explicit allowlist of offsets, not a count.

### 5. Use the inactive slot as a double buffer *(supersedes "load into the active slot")*

`PLAN.md` says Next/Prev loads into the *active* slot. `tonex.cpp:158-163` (`switchSilently`) instead writes the preset into the **inactive** slot, then switches to it — avoiding an audible artifact from reloading the slot being heard.

This fits the locked "flat list of 20, ignore A/B" decision perfectly: A/B stop being user-visible and become the double buffer that makes switching clean. Keep the flat model; change the mechanism.

### 6. Preset names are real, and here is how

`PLAN.md` cites Builty as proof names are readable but no code was known to do it; `vit3k` never implements `0x0300` at all. Builty does (`usb_tonex_one.c:240-255, 1195-1215`):

Request `0x0300` per preset, then search the response for the marker `B9 04 B9 02 BC 21`; the **32 bytes** immediately following are the name. Names are fetched **sequentially** — the response to preset *n* triggers the request for *n+1*, walking 0..19. The request's last byte selects detail level: `0x00` = ~2 KB summary (what we want), `0x01` = ~30 KB full dump.

### 7. Exact message bytes

```
Hello           b9 03 00 82 04 00 80 0b 01  b9 02 02 0b
RequestState    b9 03 00 82 06 00 80 0b 03  b9 02 81 06 03 0b
RequestPreset   b9 03 81 00 03 82 06 00 80 0b 03  b9 04 0b 01 00 00
                                                      byte[15]=index, byte[16]=detail
WriteState      b9 03 81 06 03 82 <len_lo> <len_hi> 80 0b 03  ++ <raw state body>
```

Response header types: `0x0306` = StateUpdate, `0x02` = Hello.

### 8. Open ambiguity — the `0x80` tag width

`protocol.md` says `0x80`/`0x81`/`0x82` are all u16le. But `tonex.cpp:228-247` reads `0x80` as tag **+ 1 byte** while `0x81`/`0x82` are tag **+ 2 bytes**. The hardcoded request frames are consistent with the 1-byte reading.

This only affects *parsing responses*, and it cannot be settled without a real frame. Implement the reference behaviour (`0x80` → 1 byte), gate it behind a single well-commented function, and make the fixture harness the thing that decides. This is the top item for the first capture.

### 9. Framing arrives split across reads

`tonex.cpp:165-186` accumulates bytes until a closing `0x7E` with a **1 s inter-byte timeout** that flushes a stale partial frame. The reader thread needs a resync-capable accumulator, not one frame per read.

## Build plan

### M0 — workspace

- Root `Cargo.toml` workspace, resolver 2, shared `[workspace.package]` and `[workspace.dependencies]`.
- Member crates per `PLAN.md`'s tree. This slice implements only `pinex-proto`; the rest get a `lib.rs` with a doc comment stating their intended role, so the layering is visible and `cargo check` covers the workspace.
- `rust-toolchain.toml` pinning stable.
- CI: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`. Host-native only — cross-compilation is deliberately not wired here since `cross` and Docker are absent; that is M0's remaining task on the Mac.

### `pinex-proto` — the deliverable

Pure. No I/O, no threads, no hardware, no `std::fs`. Only dependency: `crc`.

- **`frame.rs`** — `stuff`/`unstuff`, `crc16` (`CRC_16_IBM_SDLC`), `encode_frame(payload) -> Vec<u8>`, `decode_frame(&[u8]) -> Result<Vec<u8>, FrameError>`. `FrameError` distinguishes `InvalidFrame` / `InvalidEscape` / `CrcMismatch { expected, actual }` — CRC failures must be attributable, per `PLAN.md`'s fail-loudly requirement.
- **`accumulator.rs`** — byte-fed frame splitter implementing finding #9. Pure: takes bytes, emits complete frames, exposes an explicit `flush_stale()` the caller drives from its own clock. Keeps `pinex-device`'s threading free of parsing logic.
- **`value.rs`** — tagged-value decode/encode (`0x00-0x7F` literal, `0x80`/`0x81`/`0x82`, `0x88` f32, `0xB9`/`0xBA`/`0xBC` lists). Houses the finding-#8 ambiguity behind one documented function.
- **`message.rs`** — builders for the four frames in finding #7, and `parse_header` returning type/size/unknown. Validates that the declared size matches the remaining bytes and errors loudly if not.
- **`state.rs`** — `PedalState { raw: Vec<u8>, .. }` per finding #2. Accessors for the finding-#3 offsets. `set_preset_in_slot()` / `set_active_slot()` patch `raw` in place. `diff_offsets(&before, &after) -> Vec<usize>` powers the finding-#4 allowlist assertion. All bounds-checked — a short `raw` returns an error, never panics.
- **`preset.rs`** — marker scan and 32-byte name extraction (finding #6), with lossy UTF-8 decode and trailing-NUL/space trimming.

### Fixtures

Two mechanisms, since a real corpus does not exist yet:

1. **Synthesized** — the finding-#7 frames encoded through our own codec, with CRCs computed and asserted against literal expected bytes. These genuinely validate stuffing, CRC, and round-tripping, and they are checked against byte sequences that a shipping implementation transmits to real hardware. They **cannot** validate response parsing; the tests say so in comments so nobody mistakes green for proven.
2. **Directory harness** — `tests/fixtures/*.bin` loaded and decoded by a test that iterates the directory and passes trivially when empty. Dropping a capture in is the entire integration cost.

Plus a property test: `unstuff(stuff(x)) == x` for arbitrary payloads, and `decode_frame(encode_frame(x)) == x`.

### Deferred (unchanged from `PLAN.md`)

`pinex-device`, `pinex-ui`, `pinex-input`, `pinex-web`, `deploy/`, and the `cross` → rsync → systemd loop. All need hardware or a Mac. Findings #4, #5, and #9 are the notes that matter when M1/M3 resume.

## Verification

Runs here, in full:

```
cargo test --workspace      # unit + property + fixture-directory tests
cargo clippy -- -D warnings
cargo fmt --check
```

Specifically green before this is done:
- CRC of each synthesized frame equals the literal expected bytes.
- `unstuff(stuff(x)) == x` and `decode_frame(encode_frame(x)) == x` over arbitrary payloads.
- A payload containing `0x7E` and `0x7D` survives round-tripping (the stuffing path that silently corrupts if wrong).
- Corrupting one byte of a valid frame yields `CrcMismatch`, not a parse.
- The accumulator recovers a frame from bytes fed one at a time, and from two frames in one chunk.
- `set_preset_in_slot` on a synthetic `raw` changes exactly the expected offsets per `diff_offsets`, and out-of-range slots/indices error rather than panic.

**Cannot be verified here, and I will say so rather than imply otherwise:** response parsing, the finding-#8 tag width, and anything touching a pedal, Pi, display, or GPIO. The first real capture is what closes those.

## First hardware session (for later)

1. Capture one raw `RequestState` response to `tests/fixtures/` — this alone settles finding #8 and validates the CRC against reality.
2. Confirm the finding-#3 offsets against that capture before any write.
3. Re-verify in the Tonex app that nothing changed during the read-only session.
