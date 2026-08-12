# Pinex — Raspberry Pi footswitch controller + display for the IK Multimedia Tonex ONE

> **Document type:** design doc — the locked decisions and milestone map. Not an
> executable task list. Implementation plans live beside it in `docs/plans/` and
> cite it.
>
> **Status:** decisions in "Locked decisions" and "Crate selection" remain
> authoritative. **Three protocol claims below were superseded** by reading the
> reference implementations during the M0/`pinex-proto` slice — see
> [`2026-08-02-pinex-proto-scaffolding.md`](2026-08-02-pinex-proto-scaffolding.md):
>
> | Superseded here | Corrected by |
> |---|---|
> | "The central hazard" — read → parse → **re-serialize** → diff (Protocol facts; Risks) | Finding 2 — patch the raw buffer in place; never re-serialize |
> | "Next/Prev walks 1..20, loading into the **active** slot" (Locked decisions; M3) | Finding 5 — stage into the *inactive* slot, then switch |
> | "CRC variant guess is wrong" as an open risk (Crate selection; Risks) | Finding 1 — confirmed `CRC_16_IBM_SDLC`, now a test assertion |
>
> The locked *decisions* survive all three; only the mechanisms changed. Body
> text is preserved verbatim as written rather than rewritten in place.
>
> **Second update, after the hardware sessions of 2026-08-09..11.** M1, M2 and
> M3 are built and running on a Pi against a real pedal and a real panel. Two
> further claims here were overturned by hardware:
>
> | Superseded here | Corrected by |
> |---|---|
> | "Stage into the inactive slot, then switch" as the universal glitch-free path (M3) | A pedal in **stomp mode** accepts that and silently reverts it a second later. `PedalState::change_preset` writes in place there and stages only in A/B mode. Both routes are hardware-verified |
> | "`embedded-graphics` + `mipidsi` driving an **ST7789**" (M2) | The Waveshare 1.44" HAT is an **ST7735S**, 128×128, wired **BGR**, needing a `(1,2)` window offset that swaps axes with rotation |
>
> Milestone status is tabulated under "Milestones" below.

## Context

The Tonex ONE is a great-sounding amp modeller crippled by its control surface: **one** physical footswitch (toggles slot A/B) and **no display**. It stores 20 presets, but on a dark stage you cannot tell which one is loaded, and you cannot reach presets 3–20 with your foot at all.

Pinex turns a Raspberry Pi into the missing control surface: a USB host that speaks the Tonex ONE's serial protocol, shows the **current preset number and name** on a screen, and lets you **cycle through all 20 presets** with footswitches.

This is a greenfield repository — `/Users/andrewhao/workspace/pinex` is empty. No codebase exploration was possible or needed; the research that informs this plan is protocol reverse-engineering prior art (below).

**Intended outcome:** a device that boots unattended, shows the truth about what the pedal is doing, and changes presets reliably.

---

## Prior art (mine this, don't start from zero)

| Source | What to take from it |
|---|---|
| [vit3k/tonex_controller](https://github.com/vit3k/tonex_controller) | `protocol.md` — the reverse-engineered wire format. **Primary reference.** |
| [Builty/TonexOneController](https://github.com/Builty/TonexOneController) | Mature ESP32-S3/C ref implementation. Proof that all 20 preset *names* are readable at boot. Consult when our parsing disagrees with reality. |

### Protocol facts established

- Pedal enumerates as **USB CDC-ACM**, VID `0x1963` / PID `0x00D1` → appears as `/dev/ttyACM0`. On a Pi this is plain USB host; none of the ESP32 projects' USB-host-stack complexity applies.
- **HDLC-style framing:** `0x7E` flag ... payload ... CRC-16 ... `0x7E` flag, with byte-stuffing applied to **both payload and CRC**.
- **Tagged body format:** header `0xB9 0x03`, `0x81 <type u16le>`, `0x82 <size u16le>`, `0x80 <unknown u16le>`. Values are type-tagged: `0x00–0x7F` = literal small int, `0x80`/`0x81`/`0x82` = u16le, `0x88` = IEEE-754 f32. Lists use `0xB9`/`0xBA`/`0xBC`.
- **Messages:** `Hello` (handshake, returns firmware version) · `RequestState 0x0306` (full pedal state incl. active slot) · `RequestPreset 0x0300` (per-preset data, ~41 f32 per effects section).
- **⚠️ The central hazard:** there is no "set preset N" command. Changing anything requires **read full state → mutate one field → write the entire state back**. A misparsed field you never intended to touch gets written back wrong. This single fact drives the milestone ordering below.
- Tonex **firmware version affects the protocol** — the 1.8.0 release broke existing third-party controllers. Parsing must fail loudly, not silently.

---

## Locked decisions

| Decision | Choice | Rationale |
|---|---|---|
| Deployment | **Bench now, stage later** | Don't block the stage path; keep protocol decoupled from UI/GPIO. |
| Board | **Pi 3 Model B** | armv7/arm64 (no armv6 toolchain pain), 4× USB-A (no OTG adapter), full 40-pin header. Too big for a final pedalboard — form factor deferred. |
| Display | **ST7789 SPI IPS TFT** | Recommend **Pimoroni Display HAT Mini** (2.0" 320×240) or **Waveshare 1.3" LCD HAT** (240×240). Both have **onboard buttons** — the entire input→state→render loop is testable with zero soldering. |
| Preset model | **Flat list of 20; ignore A/B** | Next/Prev walks 1..20, loading into the active slot. Simplest model, matches player expectation. |
| Source of truth | **The pedal** | Pinex holds no authoritative state. It renders the pedal's broadcasts; switch presses are *requests*. The display can never lie — which is the entire point of having one. |
| Language | **Rust** | Single static binary, no runtime deps, byte-level protocol work is pleasant, good fit for a locked-down stage image later. |
| Concurrency | **Threads + channels, no async** | One reader thread, GPIO interrupts, a render loop. A runtime buys nothing. |
| Build | **`cross` on the Mac → rsync → systemd restart** | Target `aarch64-unknown-linux-gnu`. Pi 3 has 1GB RAM; native rustc linking risks OOM. |
| Protocol strategy | **Read-only first, writes later** | M1 never writes to the pedal. Zero corruption risk, and still ships a working display. |
| v1 extras | **Web debug UI only** | No bypass switch, no MIDI in, no switch-mapping config. |

### Power
Tonex ONE draws only **~125 mA** over USB-C. The Pi's port feeds it directly — no powered hub. Budget ~500 mA total (Pi + pedal + display); use a **5V/2.5A** supply. Pi 3B's USB ports sit behind a LAN9514 hub chip; irrelevant at this current.

---

## Crate selection

- **`rppal`** — GPIO + SPI. Pure Rust, no C deps, cross-compiles cleanly, interrupt-driven GPIO for switches.
- **`mipidsi`** — ST7789 driver, over `display-interface-spi`.
- **`embedded-graphics`** + **`u8g2-fonts`** — rendering; u8g2 gives large glyphs that read at arm's length.
- **`crc`** — use `CRC_16_IBM_SDLC` (poly `0x1021`, init/xorout `0xFFFF`, reflected). This is HDLC FCS-16 and is almost certainly what the doc's "CRC-CCITT" means. **Verify against a real captured frame before trusting it.**
- **`nix`** — termios raw mode.
- **`tiny_http`** — blocking HTTP, fits the thread model without dragging in tokio.

**Deliberate omission: no `serialport` crate.** CDC-ACM on Linux is just a tty and USB ignores baud rate entirely, so we open `/dev/ttyACM0` as a file and set raw mode via `nix` termios. This drops `serialport`'s `libudev` dependency — the single most painful thing to cross-compile.

> ⚠️ `rppal`, `mipidsi`, and `display-interface-spi` must agree on an `embedded-hal` major version (1.0 vs 0.2). Pin versions deliberately at M2 and treat a mismatch as expected setup work, not a surprise.

---

## Architecture

Cargo workspace. The layering exists to serve one goal: **`pinex-proto` must be pure so it can be unit-tested on the Mac with byte fixtures, with no Pi and no pedal.**

```
pinex/
├── Cargo.toml                  # workspace
├── crates/
│   ├── pinex-proto/            # PURE. No I/O, no threads, no hardware.
│   │   ├── frame.rs            #   HDLC stuff/unstuff, CRC-16, frame split
│   │   ├── value.rs            #   tagged value encode/decode (0x80/0x81/0x82/0x88)
│   │   ├── message.rs          #   Hello / RequestState / RequestPreset
│   │   ├── state.rs            #   PedalState, PresetSlot, active-slot field
│   │   └── tests/fixtures/     #   captured real frames — the regression corpus
│   ├── pinex-device/           # tty transport, reader thread, reconnect, event bus
│   ├── pinex-ui/               # embedded-graphics rendering; Display trait
│   ├── pinex-input/            # InputSource trait: GpioInput | HatButtons | StdinInput
│   ├── pinex-web/              # tiny_http debug server
│   └── pinex/                  # binary: config, wiring, systemd
└── deploy/                     # udev rule, systemd unit, deploy.sh
```

### Event model

```rust
enum PedalEvent { Connected { firmware: String }, Disconnected,
                  StateChanged(PedalState), PresetNames(Vec<String>),
                  ParseError { raw: Vec<u8>, reason: String } }

enum Command { RequestState, RequestPreset(u8), SetPreset(u8) /* M3 only */ }
```

Reader thread owns the tty and emits `PedalEvent` on a channel. Renderer and web server are **subscribers**. Input emits `Command`. Nothing but the reader thread touches the serial port.

**`ParseError` is a first-class event, not a log line.** It surfaces in the web UI with raw hex. When a Tonex firmware update breaks us, this is how we find out — loudly, immediately, with the bytes in hand.

---

## Milestones

**Status as of 2026-08-11:**

| | State |
|---|---|
| M0 — Scaffolding | ✅ workspace, cross-compilation, deployed to the Pi |
| M1 — Read-only | ✅ handshake, state, all 20 preset names, unsolicited broadcasts, debug web page, reconnect |
| M2 — Display | ✅ ST7735S over SPI, three pages, three themes |
| M3 — Footswitches + write path | ✅ write path hardware-verified in both slot modes; input is the HAT's joystick and keys rather than wired footswitches |
| M4 — Stage hardening | ⏳ deferred, as planned |

Two deviations from the plan below, both deliberate:

- **Input is the HAT's joystick and three keys**, not two wired GPIO
  footswitches. The HAT was already there and gives eight inputs; wired
  switches remain the right answer for actually standing on, and the
  `InputSource` seam means adding them touches nothing else.
- **`pinex-web` uses `std::net`, not `tiny_http`.** Same blocking accept loop,
  one fewer dependency to cross-compile.


### M0 — Scaffolding
Workspace skeleton, `cross` + Docker Desktop working, `aarch64-unknown-linux-gnu` binary rsync'd to the Pi and running. Proves the loop end to end before any real logic.

### M1 — Read-only: see the truth *(the core milestone)*
**Sends only `Hello`, `RequestState`, `RequestPreset`. Never writes.**

1. `pinex-proto`: frame codec + CRC + tagged-value decode, unit-tested on the Mac.
2. `pinex-device`: open tty raw, `Hello` handshake, log firmware version.
3. Parse `0x0306` state → extract active preset index.
4. Fetch all 20 preset names via `0x0300` at startup.
5. Listen for unsolicited broadcasts (this is what makes pedal-as-source-of-truth real).
6. `pinex-web`: debug page — connection status, firmware, current preset, all 20 names, recent decoded frames + raw hex.
7. Reconnect loop; systemd unit; udev rule.

**Exit criteria:** step on the pedal's own footswitch or change a preset in the Tonex app, and the web page updates correctly — with the pedal provably unmodified.

### M2 — Display
`pinex-ui` with `embedded-graphics` + `mipidsi`. Large preset number, preset name below, connection state. Explicit **"NO PEDAL"** screen. Confirm HAT pinout against vendor docs; enable SPI (`dtparam=spi=on`).

### M3 — Footswitches + the write path
First writes. Implement read-modify-write carefully: parse full state, mutate **only** the active slot's preset index, re-serialize, compare against the original byte-for-byte, and **assert nothing but the intended field changed** before transmitting. Wire two GPIO switches (interrupt-driven, debounced) for Next/Prev, wrapping 1↔20.

### M4 — Stage hardening *(deferred)*
Read-only rootfs, boot-time trimming, final form factor (Zero 2 W), enclosure.

---

## Linux gotchas to handle in `deploy/`

- **ModemManager will probe `/dev/ttyACM0` and send AT commands at it.** Classic CDC-ACM footgun. Blacklist the device via udev (`ENV{ID_MM_DEVICE_IGNORE}="1"`) or don't install it. Suspect this first if the handshake behaves strangely.
- **udev rule** matching `1963:00d1` → stable `/dev/tonex` symlink, so we never depend on `ttyACM0` numbering.
- Run user in the **`dialout`** group (or set udev `MODE`/`GROUP`).
- Enable SPI in `/boot/firmware/config.txt`.

---

## Risks

| Risk | Mitigation |
|---|---|
| **Clobbering pedal state via read-modify-write** | M1 is read-only. M3 diffs re-serialized state against the original and refuses to send unexpected deltas. |
| Undocumented fields in `protocol.md` | Preserve unknown bytes verbatim; never synthesize them. Fixture corpus of real frames. |
| Tonex firmware update breaks parsing | `ParseError` as a visible first-class event with raw hex, surfaced in the web UI. Record the working firmware version in the README. |
| CRC variant guess is wrong | Validate `CRC_16_IBM_SDLC` against a captured frame in M1 step 1, before building on it. |
| `embedded-hal` version conflicts across rppal/mipidsi | Pin versions deliberately at M2; budget time for it. |

---

## Verification

**On the Mac (no hardware):**
- `cargo test -p pinex-proto` — round-trip stuff/unstuff, CRC vectors, tagged-value encode/decode, and decode of every captured fixture.
- Property test: `unstuff(stuff(x)) == x` for arbitrary payloads.

**On the Pi (M1):**
- `journalctl -u pinex -f` shows handshake + firmware version.
- Web UI at `http://<pi>:8080` lists all 20 preset names.
- Press the pedal's own footswitch → web UI reflects the change within ~1s.
- Unplug USB → "Disconnected"; replug → auto-recovers without a restart.
- Confirm via the Tonex app that **no pedal setting changed** during the entire M1 session.

**On the Pi (M3):**
- Next/Prev walks 1..20 and wraps both directions.
- Display, web UI, and the pedal's actual output all agree.
- Rapid switch presses don't desync or corrupt state.
- Re-verify in the Tonex app that **only** the preset selection changed.
