# Unframed message bodies

Captured message *bodies* — no flags, no CRC. `protocol.md` prints some captures
without framing, so they cannot go in `../` where the frame harness would reject
them.

| File | Source | Validates |
|---|---|---|
| `state_changed.body.bin` | `protocol.md` § State Changed, prose annotations stripped | `state.rs` field offsets |

These are transcriptions, not our own captures. A real capture from our pedal
supersedes them.

## How `state_changed.body.bin` was produced

`extract_state_changed.py` in this directory, run against a fetch of
[`vit3k/tonex_controller` `protocol.md`](https://raw.githubusercontent.com/vit3k/tonex_controller/main/protocol.md).
It strips `[...]` annotations and keeps hex byte tokens. It is checked in so the
extraction can be re-run and compared rather than taken on trust.

Extraction check: the first value after the two list headers must be
`88 00 00 70 41`, the annotated `inputTrim` of 15.0. It lands at index 12.

## Known defect in the source dump: the declared size is stale

The header declares a body of `0x97` = 151 bytes. The dump actually carries 157.
The difference is exactly six bytes — the two fields its author marks
*"added in 1.2 firmware version"*: the 1-byte tempo source and the 5-byte tempo
float.

So this dump is a **splice**: a pre-1.2 header in front of post-1.2 trailing
fields, not one coherent capture. Consequences:

- `parse_header` rejects it. `tests/state_offsets.rs` uses
  `parse_header_unvalidated` on purpose, and says so.
- It cannot be used to validate framing or size handling. It validates *offsets*
  and nothing else.
- The offsets from the end are still trustworthy: the trailing fields are
  present and in the order the annotations give, which is what those offsets
  address.

A real capture from our own pedal replaces this file and removes the caveat.
