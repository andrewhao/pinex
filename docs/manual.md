# Pinex — User Manual

Everything you can do from the pedal's own panel, and what each control means.

## The controls

The Waveshare 1.44" HAT gives a five-way joystick and three buttons. Everything
below uses them.

**The joystick means what the screen shows.** A direction does not have one
fixed job — it does whatever matches the layout of the page you are on, because
the preset list runs down the screen while A and B sit side by side, and on the
Levels page the rows stack while their bars run across.

| Control | A/B and Stomp | Levels |
|---|---|---|
| **Joystick ↑** | Previous preset | Move up to the **VOL** row |
| **Joystick ↓** | Next preset | Move down to the **TRIM** row |
| **Joystick ←** | Edit slot **A** (the one drawn on the left) | Turn the row **down** |
| **Joystick →** | Edit slot **B** (the one drawn on the right) | Turn the row **up** |
| **Joystick press** | Apply — load the preset you have chosen | — |
| **KEY1** | Apply (same as joystick press) | — |
| **KEY2** | Change page: A/B → Stomp → Levels → A/B | |
| **KEY3** | Refresh — re-read everything from the pedal | |

Left and right are **directional, not a toggle**: ← always reaches slot A,
whichever slot you were editing, so pressing the same way twice does not send
you back where you came from.

Over a keyboard (`cargo run -p pinex`, or a terminal on the Pi): `h` `j` `k` `l`
for ← ↓ ↑ →, with `n`/`p` still accepted for ↓/↑; `s` select, `t` page,
`r` refresh, `q` quit.

## The one rule worth knowing

**Scrolling never changes your sound.** Moving through the preset list moves a
cursor; the pedal keeps playing whatever it was playing. Nothing is applied
until you **press**.

That is deliberate. A control that applies values as you pass through them makes
browsing dangerous on stage, and browsing is most of what this device is for.

The one exception is the Levels page, where the value applies as it moves —
because that is what a level knob does.

## Switching between A/B and stomp mode

Press **KEY2** to move between pages:

```
A/B  →  Stomp  →  Levels  →  A/B
```

Arriving on **A/B** puts the *pedal* into A/B mode. Arriving on **Stomp** puts
it into stomp mode. This is not only a display change: the pedal's own mode
follows, which is why its footswitch behaviour changes with it.

So from A/B, press KEY2 twice to reach Stomp (by way of Levels), and twice more
to come back.

If you change mode on the pedal itself, the panel follows within a second. It
never shows the A/B page while the pedal is in stomp mode — a page that
disagreed with the pedal would be worse than no page.

## Choosing a preset in A/B mode

The A/B page shows **both slots at once**. Bright is the one making sound; dim
is the other. An amber bar underneath marks the slot you are editing.

1. **Press ← for A or → for B** — the direction matches where the box is drawn.
2. **Joystick ↑ / ↓** to scroll through presets. That slot's box previews what
   you would get — artwork, colour and name all change as you scroll. Your
   sound does not.
3. **Press the joystick** (or KEY1) to apply.

What pressing does depends on which slot you were editing:

- **The slot that is _not_ playing** — the preset loads into it *and* the pedal
  switches to it, as a single write. No gap, no intermediate state.
- **The slot that _is_ playing** — the header shows **LIVE**, and pressing
  changes your sound immediately. That is a legitimate thing to want; the
  warning is there so it is not a surprise.

**To set up an A/B pair:** press ← for A, scroll, press. Then → for B, scroll,
press. Each assignment leaves the other slot untouched — that is asserted in
the test suite, not merely intended.

**To step between them afterwards:** the pedal's own footswitch does it, or
select the slot you want and press.

**The footswitch always wins.** Stomping moves the panel to whichever slot is
now playing and discards whatever you were part-way through choosing. The
footswitch is you telling the pedal which slot you mean, so an edit still aimed
at the slot you just left is aimed at the wrong sound — that is how "edit B,
stomp back to A, edit A" ended up writing B. Nothing has been sent to the pedal
at that point, so nothing is lost but a few presses.

You can still choose the silent slot on purpose with ← or → and stage the next
sound into it. That choice just does not survive a stomp.

## Choosing a preset in stomp mode

Stomp mode uses the pedal's third slot (C), which only exists in that mode. One
box, no A/B.

1. **KEY2** until the page reads **STOMP**.
2. **Joystick ↑ / ↓** to scroll. The box shows what you would get.
3. **Press** to load it.

The number turns green when what you are looking at is what is playing.

### The footswitch

Stomping the pedal's own footswitch is reflected on the panel. The status LED
on the drawn box goes dark and **BYPASS** appears at the top when the pedal is
bypassed; the LED lights again when you switch it back on.

The enclosure stays bright either way, because a bypassed pedal is still the
loaded one — only the light goes out, as on a real pedal. The LED lights only
while you are looking at the preset slot C actually holds: one you are merely
browsing is not switched on, and lighting it would claim a sound nobody is
hearing.

The pedal announces this itself, so the panel follows the switch without asking
it anything.

## Setting the levels

One page carries both levels, because paging is the expensive gesture on a box
with three buttons.

1. **KEY2** until the page reads **LEVELS**.
2. **Joystick ↑ / ↓** to pick which row you are editing — the rows are stacked,
   so this is the axis that moves between them.
3. **Joystick ← / →** to turn it down or up — each row is a horizontal bar, so
   this is the axis that moves along it.

| Row | What it is | Range | Step |
|---|---|---|---|
| **VOL** | Master volume — the output level, what you would ride at a venue | −40 to +3 dB | 1 dB |
| **TRIM** | Input trim — gain staging, set once to match the instrument | −15 to +15 dB | 0.5 dB |

The page opens on **VOL**, since that is the one you reach for. Both apply as
they move; there is no press. The focused row is the bright one, the tick is
unity, and a bar turns amber above unity so a boost is obvious at a glance.

**VOL may read `-- dB` briefly after connecting.** Master volume is in no state
message — the pedal only reports it when asked — so until it answers there is
no honest number to show, and nudging asks again rather than inventing a
starting point to step from.

## Reading the display

| What you see | What it means |
|---|---|
| Green number or box | This is playing |
| White number | You are looking at it, not hearing it |
| Amber bar under a slot | The slot you are editing |
| **LIVE** in the header | You are editing the slot that is making sound |
| **SENDING** in the header | Sent to the pedal, not yet confirmed |
| **NO PEDAL** | USB disconnected; it will reconnect on its own |
| Red text along the bottom | A message we could not parse, with the reason |

Preset names scroll back and forth under each slot when too long to fit. Names
that already fit do not move — motion on a stage display is a cost, and paying
it for a name that fits is a distraction.

## The look

One: moulded stompboxes, with the enclosure tinted the colour the pedal itself
lights for that preset. There is nothing to switch and no `PINEX_THEME`.

Two alternatives were built — an amp control panel whose knob angle encoded the
preset, and a big-number marquee — and both were removed. Carrying three
layouts meant three ways for every panel change to be wrong, and only one of
them was ever going to be used. Git has them if the question reopens.

## The debug page

`http://<pi>:8080/` — connection state, firmware version, every preset name
with its colour swatch, and recent frames with raw hex.

Parse failures appear there with the offending bytes. That is how a firmware
change gets diagnosed: loudly, with evidence, rather than as a pedal that
quietly stops responding.

## If something is wrong

**Panel blank, pedal working.** The app survives a dead panel deliberately —
the pedal and the web page carry on. Check `systemctl --user status pinex` for
a line starting `! panel:`.

**"NO PEDAL" with the pedal plugged in.** It retries every two seconds. Check
`ls /dev/ttyACM0`, and that the user is in the `dialout` group.

**The pedal stops answering entirely.** Observed once after roughly twenty
rapid preset fetches: silent to everything including the handshake, while still
enumerating over USB and presenting its tty. **A power cycle fixes it.** Not
yet root-caused; see the open questions in `docs/plans/README.md`.

**Wrong colours, orientation or a band of noise at an edge.** All properties of
the glass, all environment variables:

| Variable | Values | Default |
|---|---|---|
| `PINEX_PANEL_ROTATION` | `0` `90` `180` `270` | `90` |
| `PINEX_PANEL_OFFSET` | `x,y` | derived from rotation |
| `PINEX_PANEL_COLOR_ORDER` | `rgb` `bgr` | `bgr` |

Run `panel_calibrate` on the Pi to see a test pattern: a border touching every
edge, a labelled corner, and R/G/B bars. If red and blue are swapped, the
colour order is wrong; if there is noise outside the border, the offset is.

## Two things not yet verified

**The bindings above are verified on hardware.** All eight were checked press
by press with `binding_test`, which prints the whole chain — which input fired,
which event it became, what the browser did — and every one matches this table.

They were wrong for three commits before that: left and right scrolled presets,
KEY2 refreshed instead of paging, and KEY3 quit the process. Four tests now
assert the table's contents, which nothing did before.

**A direction no longer has one fixed job.** Left and right once both meant
"swap the edited slot", which put the meaning in the binding table where it
could not vary by page — and made "left and right move the value" impossible to
express, because the two were indistinguishable by the time the browser saw
them. The joystick now reports only *where it was pushed*, and each page reads
the axes to match what it draws.
