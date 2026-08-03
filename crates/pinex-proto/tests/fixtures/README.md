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
