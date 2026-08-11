# Working without hardware

Every layer has a software stand-in, so the whole system runs on a laptop with
no pedal, no Pi and no display. This is not a mock of the system — it is the
system, with the ends swapped.

| Layer | Real | Stand-in |
|---|---|---|
| Pedal | `TtyTransport` on `/dev/ttyACM0` | `PedalSim` — a PTY replaying captured pedal bytes |
| Panel | `HatDisplay` — ST7735S over SPI | `PreviewPanel` — a framebuffer that prints itself |
| Buttons | `HatButtons` — GPIO | `StdinInput` / `ScriptedInput` |
| Logic | `PresetBrowser`, `panel::draw` | the same code, both ways |

## Iterating on the screen

```sh
cargo run -p pinex --example panel_sim
```

Runs the real app loop against the simulated pedal and draws the panel into
your terminal in truecolour, two pixels per character cell. Keys are the usual
`n`/`p`/`s`/`r`/`q`. `PINEX_PREVIEW_SCALE=2` halves it for a narrow terminal.

The pixels come from `panel::draw` — the same call the ST7735S gets — so a
layout change is visible immediately, without a cross-compile, a copy, a service
restart, and someone in the room looking at the glass.

Point it at a real pedal if you have one, and only the display stays simulated:

```sh
cargo run -p pinex --example panel_sim -- /dev/cu.usbmodem201134301
```

## Iterating on the whole app

```sh
cargo run -p pinex -- --sim        # console output, simulated pedal
```

## What the harness cannot tell you

Properties of the physical glass, and nothing else:

- **Orientation** — which way up it reads depends on how the unit is mounted
- **Window offset** — the ST7735S shows 128×128 of a 132×162 memory, and the
  inset is a property of the panel
- **Colour order** — RGB versus BGR swaps red and blue

`cargo run -p pinex --features hat --example panel_calibrate` on the Pi settles
all three: it draws a border touching every edge, a labelled corner, and R/G/B
bars. Both offset and rotation are environment variables
(`PINEX_PANEL_OFFSET`, `PINEX_PANEL_ROTATION`) so finding the right values does
not need a rebuild.

## Layout regressions

`PreviewPanel::to_ascii` renders the framebuffer as plain characters, so a
layout test fails with a readable picture rather than a pixel count that moved
from 1,482 to 1,477. See
`preview::tests::the_no_pedal_screen_looks_like_this`.
