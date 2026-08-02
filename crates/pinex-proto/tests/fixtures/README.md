# Captured frame fixtures

Drop raw bytes read from the pedal's tty here as `*.bin`. `tests/fixtures.rs`
picks them up automatically — no registration step.

A file may contain one frame, several, or leading garbage; the accumulator
resyncs. Capture the bytes exactly as read, before any parsing.

```sh
# on the Pi, with the pedal connected
cat /dev/tonex > state-response.bin    # then trigger the message and ^C
```

## What to capture first

1. **A `RequestState` response.** This is the single highest-value capture. It
   settles the `0x80` tag-width question documented on
   `pinex_proto::value::tag_width`, and it validates the CRC against real
   hardware rather than against a reimplementation.
2. **A preset-details response**, to confirm the name marker and the response's
   message type code.
3. **A Hello response**, for the firmware version format.

Record the pedal's firmware version alongside any capture — the 1.8.0 release
broke third-party controllers, so a fixture without a version is hard to
interpret later.
