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

Still open, and none of it can be closed without a pedal:

- **Whether a real pedal accepts our requests.** Every request is byte-identical
  to what the reference implementations transmit, but nothing has been sent to
  hardware. The simulator cannot tell us this — it recognises our requests
  *because* they are ours.
- **Whether we parse real state responses.** The captured state body is
  transcribed and unframed, and its source dump's declared size is stale by the
  six bytes of two fields added in firmware 1.2, so it is a splice rather than
  one coherent capture. It validates field offsets and nothing else. See
  `crates/pinex-proto/tests/fixtures/bodies/README.md`.
- **`offset_from_start::STOMP_MODE`.** Inferred from position; the capture does
  not annotate that byte. The only from-start constant without direct evidence.
- **The preset-name marker and the preset response's message type code.** No
  capture at all. The simulator deliberately refuses to fabricate a reply here.
- **Anything touching a Pi, display, or GPIO.**

First hardware session should capture, in order: a `RequestState` response from
our own pedal, a preset-details response, and a Hello response to confirm the
firmware-version format has not moved since 1.1.3.
