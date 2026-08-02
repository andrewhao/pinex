# Plans

Design docs and implementation plans, named `YYYY-MM-DD-<feature-name>.md` by the
date the work was written. Newest last.

| Plan | Type | Status |
|---|---|---|
| [2026-08-01-pinex-design.md](2026-08-01-pinex-design.md) | Design doc — locked decisions, crate selection, milestones M0–M4 | Living. Three protocol claims superseded; see its status banner. |
| [2026-08-02-pinex-proto-scaffolding.md](2026-08-02-pinex-proto-scaffolding.md) | Implementation plan — M0 workspace + `pinex-proto` | ✅ Complete (`3a390ff`) |

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

- **`0x80` tag width** (`crates/pinex-proto/src/value.rs`) — `protocol.md` says
  2 bytes, both reference implementations read 1. Currently follows the
  references, isolated in `value::tag_width`, pinned by
  `message::tests::request_frames_size_field_discrepancy`. One captured
  `RequestState` response settles it; see
  `crates/pinex-proto/tests/fixtures/README.md`.
