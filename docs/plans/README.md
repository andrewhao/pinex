# Plans

Design docs and implementation plans, named `YYYY-MM-DD-<feature-name>.md` by the
date the work was written. Newest last.

| Plan | Type | Status |
|---|---|---|
| [2026-08-01-pinex-design.md](2026-08-01-pinex-design.md) | Design doc — locked decisions, crate selection, milestones M0–M4 | Living. Three protocol claims superseded; see its status banner. |
| [2026-08-02-pinex-proto-scaffolding.md](2026-08-02-pinex-proto-scaffolding.md) | Implementation plan — M0 workspace + `pinex-proto` | ✅ Complete (`3a390ff`) |
| [2026-08-03-protocol-ground-truth-and-pty-simulator.md](2026-08-03-protocol-ground-truth-and-pty-simulator.md) | Implementation plan — settle the protocol against captures, then build a PTY pedal simulator | ✅ Complete |

No plan document was written for the hardware, display and stage-UI work that
followed; it ran directly against a real pedal and a real panel, with the
findings recorded in commit messages, in `docs/protocol-metadata.md`, and in the
open questions below. That was the right call for work whose next step depended
on what the hardware said, but it does mean the commit log is the record.

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

- ~~Whether the pedal accepts our WRITES~~ — **confirmed, after correcting a
  real bug.** Preset changes were verified end to end on hardware: the browser
  switched the pedal from preset 2 to 3 and back, and the pedal reported each
  change.

  **What hardware overturned:** staging a preset into the inactive slot and
  switching to it — the approach *both* reference implementations use, and the
  one the design doc calls glitch-free — is accepted by a pedal in stomp mode
  and then silently reverted about a second later. Writing the preset into the
  *current* slot in place is what sticks, and costs one byte instead of three.
  `PedalState::change_preset` picks the route per slot. **Both routes are now
  hardware-verified.** In A/B mode all three candidate strategies stick, and
  stage-and-switch was kept there because it alone preserves the double
  buffering — successive changes alternate `[B, A, B, A]` and the slot being
  heard keeps its preset, which is what makes a change inaudible.

  A first version of the verification harness reported PASS for this: it read
  the pedal's immediate echo and returned before the revert. Confirming a write
  means letting the pedal settle and asking again.

- **Message type `0x0005`.** Five bytes, empty body, sent after a state write.
  Undocumented anywhere we have seen. Recognised as `WriteAck` so it stops
  looking like a parse failure, but nothing is inferred from it — the pedal
  sends it even when it is about to revert the change.
- **The pedal stops answering after sustained traffic.** Observed on 1.3.17:
  after capturing twenty presets back to back, the pedal went silent to
  everything including Hello, while still enumerating on USB (session ID
  unchanged) and presenting its tty. Reads returned zero bytes, writes
  succeeded, DTR/RTS were already asserted; a 1200-baud touch and the blocking
  `/dev/tty.*` node made no difference, and the latter blocked on carrier
  detect. **A power cycle fixed it** — confirmed. Bulk preset fetches may want
  pacing, and reconnect handling should assume this can happen mid-set.
- **`offset_from_start`** — removed entirely. The fields it addressed shift
  between firmwares; anything near the start of the state must be walked.
- ~~**Anything touching a Pi, display, or GPIO.**~~ — **done.** Running on a
  Pi 3 as a systemd user service against the pedal, driving a Waveshare 1.44"
  ST7735S panel with its joystick and keys. See `docs/harness.md` for how the
  same code runs with none of it attached.

**Update, hardware sessions of 2026-08-10/11.** The Pi, the panel and the stage
UI landed. What those sessions closed, and what they opened:

- ~~**Does it run on a Pi?**~~ — yes, and two bugs only the Pi could show:
  `StandardInput=null` made the service exit in ten milliseconds (systemd then
  crash-looped it), and opening the tty once meant no pedal meant no program.
- ~~**A/B slot management, stomp mode, global gain**~~ — built and verified
  end to end against the pedal. Assigning one slot never disturbs the other,
  which is asserted rather than assumed after a stale-snapshot bug where two
  sequential writes silently undid each other.
- ~~**Panel orientation, window offset, colour order**~~ — 90°, `(2,1)`, and
  **BGR**. All three are properties of the glass and all three are environment
  variables, because none can be inferred from code.

Still open:

- **Bulk preset fetches are unpaced.** Twenty back-to-back requests is what
  wedged the pedal on 2026-08-09. Nothing has been done about it; the reconnect
  loop copes if it happens, but not fetching that fast in the first place would
  be better.
- **Master volume (`0x0309`) is unimplemented.** "Global gain" is currently
  input trim, which is in the state and safe to patch. Master volume is a
  separate message documented by `Builty/TonexOneController` and never sent by
  us. See `docs/protocol-metadata.md`.
- **112 named parameters are unreachable.** Single-parameter writes would turn
  this from a preset browser into a tone controller. Deliberately out of scope
  so far — the goal said effects settings were not needed.
- **The full ~30 KB preset dump has never been captured**, so the 329 f32
  parameter values in each preset summary remain unmapped.
- **The Marquee and amp-panel themes are barely tested on the glass.** Only
  Pedalboard has had real scrutiny. Legibility at stage distance is the whole
  point of the other two and is not something the simulator can answer.
- **No enclosure, no power plan, no boot-time trimming.** M4 in the design doc,
  still deferred.
