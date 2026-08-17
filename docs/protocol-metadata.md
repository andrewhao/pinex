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
- **Full preset dump** (`PresetDetail::Full`, ~30 KB) rather than the summary.
  We have never captured one — the simulator deliberately refuses to fake it.
- `TYPE_PARAM_CHANGED`, a notification we do not yet recognise.

## Master volume — implemented

Not in the state message at all, so it is the one setting with no snapshot to
patch and no diff to assert. It has its own request and its own report.

| | |
|---|---|
| Write | type `0x0309`, `B9 03 81 09 03 82 0A 00 80 0B 03` + `B9 04 03 00 00 88 <f32>` |
| Read | type `0x030D`, `B9 03 81 0D 03 82 05 00 80 0B 03 B9 03 03 00 00` |
| Reply | type `0x0309`, marker `B9 04 03`, 2-byte LE index, `88`, `<f32>`; index `0` is master volume |

**The value on the wire is a 0..10 linear scale, not decibels.** This is the
part worth writing down: an earlier note here said only "same message, payload
marker `0x03` instead of `0x02`", which is true and hides the thing that
matters. `Builty/TonexOneController` converts on both sides —
`((db + 40) / 43) * 10` going out, `((raw / 10) * 43) - 40` coming back — so
decibels never touch the wire. Sending `-40` because it looked like a decibel
value would put it far outside a `0..10` control.

The reference clamps nowhere. We clamp to −40..+3 dB in `master_volume_to_wire`,
and fold NaN to the floor rather than letting it reach the pedal, because this
is the one parameter where a wrong number is measured in volume.

Because the pedal never volunteers this value, the app asks for it on connect
and again after every write, and displays only what came back — `-- dB` until
then, rather than a number we assumed.

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
