# Captured frame fixtures

Drop raw bytes read from the pedal's tty here as `*.bin`. `tests/fixtures.rs`
picks them up automatically — no registration step.

A file may contain one frame, several, or leading garbage; the accumulator
resyncs. Capture the bytes exactly as read, before any parsing.

```sh
# on the Pi, with the pedal connected
cat /dev/tonex > state-response.bin    # then trigger the message and ^C
```

## Provenance

| File | Source | Firmware | Validates |
|---|---|---|---|
| `hello_response.bin` | Transcribed from [`vit3k/tonex_controller` `protocol.md`](https://github.com/vit3k/tonex_controller/blob/main/protocol.md), "Example response" under *Hello* | 1.1.3 (from the frame itself) | CRC against real hardware; strict header size check on a real response |

`hello_response.bin` was **not** captured by us from a pedal. It is a
transcription of a published capture, byte-for-byte including its CRC. The CRC
validating is meaningful precisely because we did not compute it — the pedal
did.

## What to capture first

1. **A `RequestState` response.** This is the single highest-value capture. It
   settles the `0x80` tag-width question documented on
   `pinex_proto::value::tag_width`, and it validates the CRC against real
   hardware rather than against a reimplementation.
2. **A preset-details response**, to confirm the name marker and the response's
   message type code.
3. ~~A Hello response~~ — **done**, see Provenance above. A capture from *our*
   pedal is still worth taking, to confirm the firmware-version format has not
   changed since 1.1.3.

Record the pedal's firmware version alongside any capture — the 1.8.0 release
broke third-party controllers, so a fixture without a version is hard to
interpret later.

## Captures from our own pedal (firmware 1.3.17)

These came off real hardware over USB, via
`cargo run -p pinex-device --example capture`. They supersede the
transcriptions above wherever the two disagree.

| File | Request | Establishes |
|---|---|---|
| `hw_hello_fw1_3_17.bin` | `hello` | Firmware 1.3.17; Hello response shape unchanged since 1.1.3 |
| `hw_state_response.bin` | `state` | A **framed** state response with a real CRC — the capture this directory asked for. Confirms `0x80` = 1 byte a second time (`80 9f` = 159 = exact body length) and validates every end-relative offset |
| `hw_preset0_response.bin` | `preset:0` | Preset response type code `0x0304`, previously unconfirmed; name layout |
| `hw_preset15_response.bin` | `preset:15` | Name layout is stable across presets and index round-trips |

**What these changed.** Two things the earlier documents got wrong:

1. The preset response type code was unknown. It is `0x0304`.
2. Start-relative state offsets are **not** firmware-stable. 1.3.17 opens the
   state's inner list with `b9 0e` (14 elements) where 1.1.3 has `b9 0b` (11).
   Every constant offset into that region shifts silently. They have been
   removed from `state.rs`; see the note there. End-relative offsets are
   confirmed on both firmwares and are what the write path uses.
