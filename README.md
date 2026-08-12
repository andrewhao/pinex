# Pinex

A footswitch controller and display for the [IK Multimedia Tonex ONE][tonex] —
a Raspberry Pi, a small SPI panel, and enough of the pedal's USB protocol to
manage it on stage.

The Tonex ONE holds twenty presets behind a single footswitch and has no
display. Pinex adds the two things that makes it usable live: seeing what is
loaded, and choosing what is loaded next.

[tonex]: https://www.ikmultimedia.com/products/tonexone/

## What it does

- **A/B slot management** — see what is in both footswitch slots at once,
  assign either without disturbing the other, and step between them
- **Stomp mode** — reach the pedal's third slot, which only exists in that mode
- **Global gain** — input trim, applied as you turn it
- **Generated pedal artwork** — the archetype is inferred from the preset name,
  and the enclosure takes the colour the pedal itself lights for that preset
- **Three themes** — `pedalboard` (moulded stompboxes), `amp` (a control panel
  where the knob angle *is* the preset), `marquee` (big numbers)
- **A debug web page** on port 8080: connection, firmware, every preset name
  and colour, and recent frames with raw hex

## Using it

Joystick up/down scrolls presets, left/right swaps which slot you are editing,
press applies. KEY2 changes page (A/B → Stomp → Gain), KEY3 refreshes.
Scrolling never changes your sound — nothing is applied until you press.

Full details in [`docs/manual.md`](docs/manual.md).

## Running it without any hardware

Every layer has a software stand-in, so the whole system runs on a laptop with
no pedal, no Pi and no display:

```sh
cargo run -p pinex --example panel_sim    # the panel, in your terminal
cargo run -p pinex -- --sim               # console output only
```

The pedal is simulated over a PTY replaying **bytes captured from real
hardware**, and the panel renders through the same `panel::draw` the ST7735S
gets. See [`docs/harness.md`](docs/harness.md).

## Running it on a Pi

```sh
./deploy/deploy.sh pi@raspberrypi.local
```

Detects 32- vs 64-bit userland, builds the matching target, installs the binary,
udev rule and service. See [`deploy/README.md`](deploy/README.md).

## Layout

| Crate | What | Hardware? |
|---|---|---|
| `pinex-proto` | Frame codec, CRC, tagged values, pedal state | none — pure, fixture-tested |
| `pinex-device` | tty transport, reader thread, event bus, PTY simulator | tty only |
| `pinex-ui` | Browser state machine, panel drawing, themes, ST7735S driver | driver behind `--features hat` |
| `pinex-input` | Input trait, keyboard, HAT buttons | buttons behind `--features hat` |
| `pinex-web` | Debug page | none |
| `pinex` | The binary and the app loop | wiring |

The rule throughout: anything that touches hardware sits behind a trait with a
fake on the other side, and everything above it is tested without the hardware.

## What is known, and how

The protocol was reverse-engineered from two published implementations and then
**checked against a real pedal**, which overturned several published claims —
including the preset-change strategy both reference implementations use, which a
pedal in stomp mode silently reverts.

- [`docs/manual.md`](docs/manual.md) — every control, and how to switch modes,
  choose a preset and apply it
- [`docs/protocol-metadata.md`](docs/protocol-metadata.md) — what the pedal
  exposes, what we read, and what we deliberately do not fabricate
- [`docs/plans/README.md`](docs/plans/README.md) — milestone status and the
  open questions, including what is still unverified
- `crates/pinex-proto/tests/hardware_captures.rs` — assertions against bytes
  from an actual pedal, firmware 1.3.17

## Status

M0–M3 complete and running on a Pi 3 against a real pedal. M4 (stage hardening,
enclosure) deferred. Written with Claude Code.
