# Plans

Design docs and implementation plans, named `YYYY-MM-DD-<feature-name>.md` by the
date the work was written. Newest last.

| Plan | Type | Status |
|---|---|---|
| [2026-08-01-pinex-design.md](2026-08-01-pinex-design.md) | Design doc — locked decisions, crate selection, milestones M0–M4 | Living. Three protocol claims superseded; see its status banner. |
| [2026-08-02-pinex-proto-scaffolding.md](2026-08-02-pinex-proto-scaffolding.md) | Implementation plan — M0 workspace + `pinex-proto` | ✅ Complete (`3a390ff`) |
| [2026-08-03-protocol-ground-truth-and-pty-simulator.md](2026-08-03-protocol-ground-truth-and-pty-simulator.md) | Implementation plan — settle the protocol against captures, then build a PTY pedal simulator | ✅ Complete |

## Conventions

- **Design docs** hold decisions and rationale. They are amended, not replaced —
  when a decision is overturned, a banner at the top says so and points at the
  plan that overturned it, rather than the body being silently rewritten.
- **Implementation plans** are executable task lists per
  `superpowers:writing-plans`, and carry a `> **For Claude:**` directive at the
  top saying whether to execute them. Completed ones are kept for the reasoning,
  marked so they are not re-run.
- A plan that contradicts an earlier document must say which claim it supersedes
  and why, in the document itself. Reconstructing that from git history is the
  failure mode this structure exists to prevent.

## Open questions carried forward

Resolved by the 2026-08-03 plan, kept here only to say where the answers live:

- ~~**`0x80` tag width**~~ — settled at **1 byte** from published captures. The
  evidence and the reason `protocol.md`'s prose is wrong are on
  `pinex_proto::value::tag_width`. The CRC was validated against real hardware
  bytes at the same time.

**Update, hardware session of 2026-08-09.** A real ToneX One (firmware 1.3.17)
was connected and three of these were closed outright — see
`crates/pinex-proto/tests/hardware_captures.rs`. What follows is what is still
open *after* that session.

Still open:

- ~~Whether a real pedal accepts our *read* requests~~ — **confirmed.** Hello,
  RequestState and all twenty RequestPreset requests were transmitted to a real
  pedal and answered correctly.
- ~~Whether we parse real state responses~~ — **confirmed.** A genuinely framed
  state response with a real CRC now exists and every end-relative offset
  validates against it.
- ~~The preset-name marker and the preset response type code~~ — **confirmed.**
  Type `0x0304`; the name is a 33-slot buffer followed by its true length.

- **Whether the pedal accepts our WRITES. Not verified.** This is the one gap
  that matters. `message::set_preset` is built from the pedal's own state bytes
  and refuses to transmit if any byte outside the intended three would change,
  and that logic is tested against real captured state — but no write has ever
  been transmitted to hardware. The simulator accepts them because it is ours.
- **The pedal stops answering after sustained traffic.** Observed on 1.3.17:
  after capturing twenty presets back to back, the pedal went silent to
  everything including Hello, while still enumerating on USB and presenting its
  tty. Reads return zero bytes, writes succeed, DTR/RTS are already asserted, and
  the device node re-enumerates periodically. A power cycle is the suspected fix
  but is unconfirmed. Bulk preset fetches may need pacing.
- **`offset_from_start`** — removed entirely. The fields it addressed shift
  between firmwares; anything near the start of the state must be walked.
- **Anything touching a Pi, display, or GPIO.** The seams exist and are tested
  with fakes; only `GpioInput` and the SPI panel are unwritten.
