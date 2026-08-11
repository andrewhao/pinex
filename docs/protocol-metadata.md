# What else the pedal will tell us

Cross-referenced between our own captures (firmware 1.3.17) and the two public
reverse-engineering efforts. Everything marked *read* below has been decoded
from bytes our pedal actually sent.

## Already used

| Field | Where | Notes |
|---|---|---|
| Firmware version | Hello, element 3 | `[1, 3, 17]` → "1.3.17" |
| Preset names | Preset response `0x0304` | 33-slot buffer + true length |
| Active slot / per-slot preset | State, end-relative | The write path depends on these |
| A4 tuning reference | State, `len-9` | 440 Hz |
| Tempo (BPM) + source | State, past the list end | 120.0, global |
| Direct monitoring | State, `len-7` | Forced on with every write |
| **Per-preset RGB colours** | State, `0xBA` list of 20 triples | Now read; mirrored on the debug page |

## Read but unused

| Field | Where | Value on our pedal |
|---|---|---|
| Device identifier | Hello, element 4 | 20 bytes, `e9 99 a1 38 …` — matches nothing printed on the case |
| Version-ish triple | Hello, element 2 | `[2, 0, 0]` — protocol or hardware revision |
| Unknowns | Hello, elements 0/1/5/6 | `0`, `199`, `14103`, `3` |
| Preset parameter block | Preset response | 329 f32 values — the full amp/cab/EQ model |
| Input trim, cab bypass, stomp mode, tuning mode | State, start-relative | See the offset warning below |

## Capabilities we have not implemented

From `Builty/TonexOneController`, marked there as needing firmware new enough to
have Editor support (ours qualifies):

- **Set a single parameter** without a whole-state write. Message type `0x0309`,
  payload `B9 04 02 00 <index> 88 <f32>`. 112 named parameters — noise gate,
  compressor, EQ, amp model gain/volume/presence/depth, cabinet and mic
  placement, reverb, modulation, delay.
- **Set master volume.** Same message, payload marker `0x03` instead of `0x02`.
- **Full preset dump** (`PresetDetail::Full`, ~30 KB) rather than the summary.
  We have never captured one — the simulator deliberately refuses to fake it.
- `TYPE_PARAM_CHANGED`, a notification we do not yet recognise.

## The start-relative offset trap

Both reference implementations use constants counted from the start of the state
body (`COLORS = 22`, `INPUT_TRIM = 15`, …). **Those values are correct for
firmware 1.3.17 and wrong for 1.1.3** — the enclosing list grew from 11 elements
to 14. They look right against whichever pedal you own and break for someone
else's.

We locate that region by shape instead: `PedalState::preset_colors` finds the
`0xBA` list of exactly 20 triples, and reads both firmware generations with the
same code. Anything else added from that region should do likewise.

## Sources

- `vit3k/tonex_controller` — `protocol.md`, the original framing/CRC/value work
- `Builty/TonexOneController` — `tonex_params.h` (112 named parameters),
  `usb_tonex_one.c` (single-parameter and master-volume messages, offsets)
