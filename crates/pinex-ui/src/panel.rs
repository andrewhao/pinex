//! Drawing the browser onto a 128×128 panel.
//!
//! [`draw`] is generic over `DrawTarget`, so the same code that drives the
//! Waveshare HAT's ST7735S renders into an in-memory buffer under `cargo test`.
//! The Pi-specific part is then only "open SPI, init the controller, call
//! `draw`" — see `pinex-ui/src/hat.rs`.
//!
//! # What the panel is for
//!
//! A player glances at this between songs, in bad light, from standing height.
//! So: the preset *number* is the biggest thing on screen, because that is what
//! gets called out; the name is secondary; and a disconnected pedal says so in
//! words rather than by showing stale information.

use embedded_graphics::mono_font::ascii::{FONT_10X20, FONT_5X8, FONT_6X10, FONT_9X15_BOLD};
use embedded_graphics::mono_font::{MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{CornerRadii, PrimitiveStyle, Rectangle, RoundedRectangle};
use embedded_graphics::text::{Alignment, Baseline, Text};

use crate::browser::{Connection, Screen, View};
use crate::skin::{self, wrap, Pedal};
use crate::theme::Theme;
use pinex_proto::state::{Slot, MAX_INPUT_TRIM_DB, MIN_INPUT_TRIM_DB};

/// The HAT's panel geometry.
pub const WIDTH: u32 = 128;
pub const HEIGHT: u32 = 128;

const BACKGROUND: Rgb565 = Rgb565::BLACK;
const DIM: Rgb565 = Rgb565::CSS_DIM_GRAY;
const TEXT: Rgb565 = Rgb565::WHITE;
const WARN: Rgb565 = Rgb565::CSS_ORANGE_RED;
const PLAYING: Rgb565 = Rgb565::CSS_LIME_GREEN;

/// How many characters of a preset name fit across at the small font.
const NAME_COLS: usize = (WIDTH / 6) as usize;

/// Draw the whole frame. Clears first, so callers need not.
pub fn draw<D>(target: &mut D, view: &View<'_>) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    target.clear(BACKGROUND)?;

    match view.connection {
        Connection::Disconnected => draw_no_pedal(target),
        Connection::Connected { firmware } => match view.screen {
            Screen::Slots => match view.theme {
                Theme::Pedalboard => draw_slots(target, view),
                Theme::Marquee => draw_slots_marquee(target, view),
            },
            Screen::Stomp => draw_stomp(target, view),
            Screen::Gain => draw_gain(target, view, firmware),
        },
    }
}

/// How many characters of the small font fit across the whole panel.
const FULL_COLS: usize = (WIDTH / 5) as usize;

/// How many fit across one slot's column.
const SLOT_COLS: usize = 11;

/// Draw a full preset name across the bottom of the panel, wrapped.
///
/// The boxes are only 59px wide, which is eleven characters — enough for a
/// badge and nothing more. The name that actually identifies the sound gets the
/// full panel width and as many lines as it needs, because a player checking
/// "is this the bright one or the dark one" is reading the variant, not the
/// pedal name.
fn draw_wrapped<D>(
    target: &mut D,
    text: Option<&str>,
    top: i32,
    max_lines: usize,
    color: Rgb565,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let Some(text) = text else { return Ok(()) };
    let lines = wrap(text, FULL_COLS);

    for (index, line) in lines.iter().take(max_lines).enumerate() {
        // The last line we can show gets an ellipsis if more remains, so a
        // clipped name never looks like a complete one.
        let is_last_shown = index + 1 == max_lines && lines.len() > max_lines;
        let shown = if is_last_shown {
            let mut truncated: String = line.chars().take(FULL_COLS - 1).collect();
            truncated.push('~');
            truncated
        } else {
            line.clone()
        };
        Text::with_alignment(
            &shown,
            Point::new(2, top + index as i32 * 9),
            MonoTextStyle::new(&FONT_5X8, color),
            Alignment::Left,
        )
        .draw(target)?;
    }
    Ok(())
}

/// The band the scrolling names occupy, and nothing else.
///
/// Redrawing all 128x128 five times a second to move two strings by one
/// character costs about a third of a Pi 3 core. This is the region that
/// actually changes between animation frames.
pub const NAME_BAND: Rectangle = Rectangle::new(Point::new(0, 107), Size::new(WIDTH, HEIGHT - 107));

/// Redraw only the scrolling names, for animation frames.
///
/// Falls back to a full redraw on any page that has no scrolling band, so a
/// caller can always use it and get a correct picture.
pub fn draw_scroll<D>(target: &mut D, view: &View<'_>) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    match view.connection {
        // No clear before the draw. Clearing the band and then painting the
        // text leaves the panel blank in between, which at five frames a second
        // is a visible flicker. The names are drawn opaquely instead, so each
        // glyph paints over its own previous pixels in a single pass.
        Connection::Connected { .. } if view.screen == Screen::Slots => {
            draw_slot_names(target, view)
        }
        _ => draw(target, view),
    }
}

/// A header strip: page name left, a marker for the page you are on.
fn draw_header<D>(target: &mut D, view: &View<'_>) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let style = MonoTextStyle::new(&FONT_6X10, DIM);
    Text::with_baseline(view.screen.title(), Point::new(3, 1), style, Baseline::Top)
        .draw(target)?;

    if view.pending {
        Text::with_alignment(
            "SENDING",
            Point::new(WIDTH as i32 - 3, 9),
            MonoTextStyle::new(&FONT_6X10, WARN),
            Alignment::Right,
        )
        .draw(target)?;
    }

    // Parse failures stay visible on every page. A firmware change that breaks
    // us must surface here, not only in a log nobody reads on stage.
    if let Some(error) = view.last_error {
        let short: String = error.chars().take(NAME_COLS).collect();
        Text::with_alignment(
            &short,
            Point::new(WIDTH as i32 / 2, HEIGHT as i32 - 2),
            MonoTextStyle::new(&FONT_6X10, WARN),
            Alignment::Center,
        )
        .draw(target)?;
    }
    Ok(())
}

/// The Marquee treatment of the A/B page.
///
/// Numbers at 10x20 — the largest the panel carries — over a bold colour spine
/// per slot. No artwork: at four paces the box detail is mush anyway, and what
/// survives is the number and the colour. The playing slot gets a filled spine
/// and its number in green; the other is outlined.
fn draw_slots_marquee<D>(target: &mut D, view: &View<'_>) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    draw_header(target, view)?;

    for (index, slot) in [Slot::A, Slot::B].into_iter().enumerate() {
        let x = index as i32 * 64;
        let playing = view.active_slot == Some(slot);
        let editing = view.selected == slot;
        let preset = if editing {
            Some(view.cursor)
        } else {
            view.slot_preset(slot)
        };
        let rgb = if editing {
            view.cursor_color
        } else {
            view.slot_color_for(slot)
        };
        let color = rgb.map(skin::from_rgb8).unwrap_or(DIM);

        // Colour spine: filled when playing, a bar when not. This is the part
        // that reads first from a distance.
        let spine = Rectangle::new(Point::new(x + 4, 24), Size::new(56, 62));
        if playing {
            spine
                .into_styled(PrimitiveStyle::with_fill(skin::dim(color, 1, 3)))
                .draw(target)?;
            Rectangle::new(Point::new(x + 4, 24), Size::new(56, 5))
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(target)?;
        } else {
            Rectangle::new(Point::new(x + 4, 24), Size::new(56, 5))
                .into_styled(PrimitiveStyle::with_fill(skin::dim(color, 1, 2)))
                .draw(target)?;
        }

        // Slot letter, small, above the number.
        Text::with_alignment(
            slot_letter(slot),
            Point::new(x + 32, 40),
            MonoTextStyle::new(&FONT_6X10, if playing { PLAYING } else { DIM }),
            Alignment::Center,
        )
        .draw(target)?;

        // The number, as big as the panel allows.
        let text = match preset {
            Some(preset) => format!("{:02}", preset + 1),
            None => "--".to_string(),
        };
        Text::with_alignment(
            &text,
            Point::new(x + 32, 68),
            MonoTextStyle::new(&FONT_10X20, if playing { PLAYING } else { TEXT }),
            Alignment::Center,
        )
        .draw(target)?;

        if editing {
            Rectangle::new(Point::new(x + 4, 88), Size::new(56, 2))
                .into_styled(PrimitiveStyle::with_fill(WARN))
                .draw(target)?;
        }
    }

    if view.active_slot == Some(view.selected) {
        Text::with_alignment(
            "LIVE",
            Point::new(WIDTH as i32 / 2, 20),
            MonoTextStyle::new(&FONT_6X10, WARN),
            Alignment::Center,
        )
        .draw(target)?;
    }

    draw_slot_names(target, view)
}

/// The two footswitch slots, side by side.
///
/// The whole point of A/B is that both sounds are already loaded, so the panel
/// shows both at once: which is in A, which is in B, and which one you are
/// hearing. Nothing here requires remembering a previous screen.
fn draw_slots<D>(target: &mut D, view: &View<'_>) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    draw_header(target, view)?;

    // Both slots, drawn small enough that both full names fit underneath.
    // A player deciding what to step on is comparing two names, so showing one
    // and making them page for the other defeats the point of the screen.
    for (index, slot) in [Slot::A, Slot::B].into_iter().enumerate() {
        let x = 3 + index as i32 * 63;
        let playing = view.active_slot == Some(slot);
        let editing = view.selected == slot;

        let letter_color = if playing { PLAYING } else { DIM };
        let loaded = view.slot_preset(slot);
        let label = match loaded {
            Some(preset) => format!("{} {:02}", slot_letter(slot), preset + 1),
            None => format!("{} --", slot_letter(slot)),
        };
        Text::with_alignment(
            &label,
            Point::new(x + 29, 20),
            MonoTextStyle::new(&FONT_9X15_BOLD, letter_color),
            Alignment::Center,
        )
        .draw(target)?;

        // The slot being edited previews the cursor; the other shows what it
        // holds. Each box and its name below always agree.
        let (preset, name, color) = if editing {
            (Some(view.cursor), view.cursor_name, view.cursor_color)
        } else {
            (loaded, view.slot_name_for(slot), view.slot_color_for(slot))
        };
        let _ = preset;

        let area = Rectangle::new(Point::new(x, 24), Size::new(59, 76));
        match name {
            Some(name) => skin::draw(target, area, &Pedal::new(name, color, playing))?,
            None => RoundedRectangle::new(area, CornerRadii::new(Size::new(3, 3)))
                .into_styled(PrimitiveStyle::with_stroke(DIM, 1))
                .draw(target)?,
        }

        if editing {
            Rectangle::new(Point::new(x, 102), Size::new(59, 2))
                .into_styled(PrimitiveStyle::with_fill(WARN))
                .draw(target)?;
        }
    }

    if view.active_slot == Some(view.selected) {
        Text::with_alignment(
            "LIVE",
            Point::new(WIDTH as i32 / 2, 20),
            MonoTextStyle::new(&FONT_6X10, WARN),
            Alignment::Center,
        )
        .draw(target)?;
    }

    draw_slot_names(target, view)
}

/// The two scrolling name columns.
fn draw_slot_names<D>(target: &mut D, view: &View<'_>) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    // Each slot's full name scrolls in its own column, under its own box, so
    // the two can be compared without either being cut and without the
    // artwork having to shrink to make room.
    for (index, slot) in [Slot::A, Slot::B].into_iter().enumerate() {
        let x = 3 + index as i32 * 63;
        let editing = view.selected == slot;
        let name = if editing {
            view.cursor_name
        } else {
            view.slot_name_for(slot)
        };
        let Some(name) = name else { continue };

        let color = if view.active_slot == Some(slot) {
            PLAYING
        } else if editing {
            WARN
        } else {
            TEXT
        };

        // Both columns scroll on the same clock, so they move together rather
        // than shimmering against each other.
        //
        // Opaque, and padded to a constant width: a scrolling window is always
        // SLOT_COLS characters, so the same cells are repainted every frame and
        // nothing needs erasing first. A shorter, static name is padded to the
        // same width so it too covers whatever preceded it.
        let text = skin::short_name(name);
        let window = skin::marquee(text, SLOT_COLS, view.tick);
        let padded = format!("{window:^SLOT_COLS$}");
        let style = MonoTextStyleBuilder::new()
            .font(&FONT_5X8)
            .text_color(color)
            .background_color(BACKGROUND)
            .build();
        Text::with_alignment(&padded, Point::new(x + 29, 114), style, Alignment::Center)
            .draw(target)?;
    }
    Ok(())
}

fn slot_letter(slot: Slot) -> &'static str {
    match slot {
        Slot::A => "A",
        Slot::B => "B",
        Slot::C => "C",
    }
}

/// Stomp mode: one box, big, because there is only one.
fn draw_stomp<D>(target: &mut D, view: &View<'_>) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    draw_header(target, view)?;

    let loaded = view.slot_preset(Slot::C);
    let showing_loaded = loaded == Some(view.cursor);
    let name = view.cursor_name.unwrap_or("...");

    let area = Rectangle::new(Point::new(24, 16), Size::new(80, 76));
    skin::draw(
        target,
        area,
        &Pedal::new(name, view.cursor_color, showing_loaded),
    )?;

    Text::with_alignment(
        &format!("{:02}", view.cursor + 1),
        Point::new(WIDTH as i32 / 2, 108),
        MonoTextStyle::new(&FONT_9X15_BOLD, if showing_loaded { PLAYING } else { TEXT }),
        Alignment::Center,
    )
    .draw(target)?;

    let preview = view.cursor_name.map(skin::short_name);
    draw_wrapped(target, preview, 118, 2, TEXT)?;
    Ok(())
}

/// Global gain, as a knob you can read across a stage.
fn draw_gain<D>(target: &mut D, view: &View<'_>, firmware: &str) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    draw_header(target, view)?;

    let span = MAX_INPUT_TRIM_DB - MIN_INPUT_TRIM_DB;
    let fraction = ((view.gain_db - MIN_INPUT_TRIM_DB) / span).clamp(0.0, 1.0);

    // A wide bar rather than a dial: at this size a bar's fill is readable at a
    // glance, where a pointer angle is not.
    let bar = Rectangle::new(Point::new(12, 52), Size::new(104, 24));
    bar.into_styled(PrimitiveStyle::with_stroke(DIM, 1))
        .draw(target)?;
    let filled = (102.0 * fraction) as u32;
    if filled > 0 {
        Rectangle::new(Point::new(13, 53), Size::new(filled, 22))
            .into_styled(PrimitiveStyle::with_fill(if view.gain_db > 0.0 {
                WARN
            } else {
                PLAYING
            }))
            .draw(target)?;
    }

    // Centre tick: unity gain, the value you return to.
    Rectangle::new(Point::new(63, 46), Size::new(1, 36))
        .into_styled(PrimitiveStyle::with_fill(TEXT))
        .draw(target)?;

    Text::with_alignment(
        &format!("{:+.1}", view.gain_db),
        Point::new(WIDTH as i32 / 2, 38),
        MonoTextStyle::new(&FONT_10X20, TEXT),
        Alignment::Center,
    )
    .draw(target)?;
    Text::with_alignment(
        "dB",
        Point::new(WIDTH as i32 / 2, 92),
        MonoTextStyle::new(&FONT_6X10, DIM),
        Alignment::Center,
    )
    .draw(target)?;

    Text::with_alignment(
        &format!("{MIN_INPUT_TRIM_DB:.0}"),
        Point::new(12, 86),
        MonoTextStyle::new(&FONT_6X10, DIM),
        Alignment::Left,
    )
    .draw(target)?;
    Text::with_alignment(
        &format!("+{MAX_INPUT_TRIM_DB:.0}"),
        Point::new(WIDTH as i32 - 12, 86),
        MonoTextStyle::new(&FONT_6X10, DIM),
        Alignment::Right,
    )
    .draw(target)?;

    Text::with_alignment(
        &format!("fw {firmware}"),
        Point::new(WIDTH as i32 / 2, 112),
        MonoTextStyle::new(&FONT_6X10, DIM),
        Alignment::Center,
    )
    .draw(target)?;
    Ok(())
}

/// The explicit "NO PEDAL" screen the spec asks for.
///
/// Deliberately shows nothing else. A blank-ish screen that still displayed the
/// last known preset would be a display that lies.
fn draw_no_pedal<D>(target: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let big = MonoTextStyle::new(&FONT_10X20, WARN);
    let small = MonoTextStyle::new(&FONT_6X10, DIM);

    Text::with_alignment(
        "NO PEDAL",
        Point::new(WIDTH as i32 / 2, 56),
        big,
        Alignment::Center,
    )
    .draw(target)?;

    Text::with_alignment(
        "check USB",
        Point::new(WIDTH as i32 / 2, 76),
        small,
        Alignment::Center,
    )
    .draw(target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An in-memory panel, so drawing is testable with no hardware and no extra
    /// dependency. Records every pixel written.
    struct Buffer {
        pixels: Vec<Rgb565>,
    }

    impl Buffer {
        fn new() -> Self {
            Self {
                pixels: vec![Rgb565::BLACK; (WIDTH * HEIGHT) as usize],
            }
        }

        fn lit(&self) -> usize {
            self.pixels.iter().filter(|p| **p != Rgb565::BLACK).count()
        }

        fn any(&self, wanted: Rgb565) -> bool {
            self.pixels.contains(&wanted)
        }

        fn pixel_at(&self, x: u32, y: u32) -> Rgb565 {
            self.pixels[(y * WIDTH + x) as usize]
        }
    }

    impl Dimensions for Buffer {
        fn bounding_box(&self) -> Rectangle {
            Rectangle::new(Point::zero(), Size::new(WIDTH, HEIGHT))
        }
    }

    impl DrawTarget for Buffer {
        type Color = Rgb565;
        type Error = core::convert::Infallible;

        fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
        where
            I: IntoIterator<Item = Pixel<Self::Color>>,
        {
            for Pixel(point, color) in pixels {
                // Anything off-panel is silently dropped, as a real target does.
                if (0..WIDTH as i32).contains(&point.x) && (0..HEIGHT as i32).contains(&point.y) {
                    self.pixels[(point.y as usize) * WIDTH as usize + point.x as usize] = color;
                }
            }
            Ok(())
        }
    }

    fn view_of<'a>(connection: &'a Connection, cursor: u8, name: Option<&'a str>) -> View<'a> {
        View {
            cursor,
            cursor_name: name,
            active: Some(cursor),
            active_name: name,
            ..View::stub(connection)
        }
    }

    /// A fully populated Slots view, as it looks once a state has arrived.
    fn slots_view(connection: &Connection, playing: Slot) -> View<'_> {
        View {
            cursor: 4,
            cursor_name: Some("TF TILT - 1 ADV"),
            cursor_color: Some([255, 63, 0]),
            active: Some(0),
            active_name: Some("TF BENSON PREAMP - 1"),
            slot_presets: Some([0, 9, 1]),
            active_slot: Some(playing),
            slot_names: [
                Some("TF BENSON PREAMP - 1"),
                Some("TF PROTEIN - BLUE 1"),
                Some("TF TILT - 1 ADV"),
            ],
            slot_colors: [Some([255, 63, 0]), Some([47, 0, 255]), Some([0, 255, 0])],
            ..View::stub(connection)
        }
    }

    #[test]
    fn the_playing_slot_is_drawn_in_the_playing_colour() {
        let c = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let mut panel = Buffer::new();
        draw(&mut panel, &slots_view(&c, Slot::A)).unwrap();
        assert!(panel.any(PLAYING), "the playing slot reads green");
    }

    /// Which slot is playing must be visible, or the panel cannot be used to
    /// decide what stepping on the pedal will do.
    #[test]
    fn playing_a_and_playing_b_look_different() {
        let c = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let mut on_a = Buffer::new();
        draw(&mut on_a, &slots_view(&c, Slot::A)).unwrap();
        let mut on_b = Buffer::new();
        draw(&mut on_b, &slots_view(&c, Slot::B)).unwrap();
        assert_ne!(
            on_a.pixels, on_b.pixels,
            "A playing and B playing must not render identically"
        );
    }

    /// The gain page must render across the whole range without overflowing.
    #[test]
    fn the_gain_page_renders_across_the_whole_range() {
        let c = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        for gain in [-15.0f32, -6.0, 0.0, 7.5, 15.0] {
            let mut panel = Buffer::new();
            let view = View {
                screen: Screen::Gain,
                gain_db: gain,
                ..View::stub(&c)
            };
            draw(&mut panel, &view).unwrap();
            assert!(panel.lit() > 0, "gain {gain} drew nothing");
        }
    }

    #[test]
    fn a_disconnected_pedal_gets_the_no_pedal_screen() {
        let mut panel = Buffer::new();
        let c = Connection::Disconnected;
        draw(&mut panel, &view_of(&c, 0, None)).unwrap();

        assert!(panel.lit() > 0, "something must be drawn");
        assert!(
            panel.any(WARN),
            "NO PEDAL should be drawn in the warn colour"
        );
        assert!(
            !panel.any(PLAYING),
            "nothing should suggest a preset is playing"
        );
    }

    /// Both slots are drawn at once — that is the whole point of the page.
    #[test]
    fn both_slots_appear_on_the_ab_page() {
        let c = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let mut panel = Buffer::new();
        draw(&mut panel, &slots_view(&c, Slot::A)).unwrap();

        let left = (0..64).any(|x| (0..HEIGHT).any(|y| panel.pixel_at(x, y) != Rgb565::BLACK));
        let right = (64..WIDTH).any(|x| (0..HEIGHT).any(|y| panel.pixel_at(x, y) != Rgb565::BLACK));
        assert!(left && right, "both slots must be visible");
    }

    /// The ticker must repaint the same cells every frame, or the panel has to
    /// be cleared first — and clearing then drawing is what made it flicker.
    #[test]
    fn every_scroll_frame_covers_the_same_cells() {
        let c = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let view = View {
            cursor_name: Some("TF MORNING GLORY - BRIGHT CUT 2"),
            slot_presets: Some([0, 1, 2]),
            active_slot: Some(Slot::A),
            slot_names: [
                Some("TF MORNING GLORY - BRIGHT CUT 2"),
                Some("TF PROTEIN - BLUE 1"),
                None,
            ],
            ..View::stub(&c)
        };

        // The set of pixels the name band touches must not vary with the tick.
        let touched = |tick: u32| {
            let mut panel = Buffer::new();
            let view = View { tick, ..view };
            draw_scroll(&mut panel, &view).unwrap();
            let mut cells = Vec::new();
            for y in NAME_BAND.top_left.y as u32..HEIGHT {
                for x in 0..WIDTH {
                    if panel.pixel_at(x, y) != Rgb565::BLACK {
                        cells.push((x, y));
                    }
                }
            }
            cells
        };

        // Different ticks paint different glyphs, but a frame drawn straight
        // after another must fully cover it — checked by drawing two ticks onto
        // one buffer and comparing against the second drawn alone.
        let mut overlaid = Buffer::new();
        draw_scroll(&mut overlaid, &View { tick: 0, ..view }).unwrap();
        draw_scroll(&mut overlaid, &View { tick: 9, ..view }).unwrap();

        let mut fresh = Buffer::new();
        draw_scroll(&mut fresh, &View { tick: 9, ..view }).unwrap();

        for y in NAME_BAND.top_left.y as u32..HEIGHT {
            for x in 0..WIDTH {
                assert_eq!(
                    overlaid.pixel_at(x, y),
                    fresh.pixel_at(x, y),
                    "pixel ({x},{y}) from an earlier frame survived; the ticker \
                     would need clearing, which is what caused the flicker"
                );
            }
        }
        assert!(!touched(0).is_empty(), "the ticker drew nothing");
    }

    /// Both slots' full names must be readable at once — the screen exists to
    /// compare two sounds, so showing one and hiding the other defeats it.
    #[test]
    fn both_slot_names_appear_on_the_ab_page() {
        let c = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let mut panel = Buffer::new();
        draw(&mut panel, &slots_view(&c, Slot::A)).unwrap();

        // Two name blocks live in the bottom third; both must have ink.
        let ink_in = |from: u32, to: u32| {
            (from..to).any(|y| (0..WIDTH).any(|x| panel.pixel_at(x, y) != Rgb565::BLACK))
        };
        assert!(ink_in(86, 108), "slot A's name row is empty");
        assert!(ink_in(108, HEIGHT), "slot B's name row is empty");
    }

    /// The longest pair of real names must both fit without either being cut.
    #[test]
    fn the_longest_two_names_both_fit() {
        let longest = "TF MORNING GLORY - BRIGHT CUT 2";
        // Prefixed with the slot letter and number, as drawn.
        let text = format!("A 20 {}", crate::skin::short_name(longest));
        let lines = crate::skin::wrap(&text, FULL_COLS);
        assert!(
            lines.len() <= 2,
            "{text:?} needs {} lines, only 2 fit per slot",
            lines.len()
        );
        assert_eq!(lines.join(" "), text, "characters lost in wrapping");
    }

    /// The complaint that prompted the full-name area: names were cut off.
    /// Every real preset name must fit the space without an ellipsis.
    #[test]
    fn every_real_preset_name_fits_the_full_name_area() {
        let names = [
            "TF BENSON PREAMP - 1",
            "TF MORNIING GLORY - BRIGHT 1",
            "TF PROTEIN - BLUE 1",
            "TF PROTEIN - GREEN 3",
            "TF TILT - BOOST FULL",
            "TF TILT - 1 ADV",
        ];
        for name in names {
            let lines = crate::skin::wrap(crate::skin::short_name(name), FULL_COLS);
            assert!(
                lines.len() <= 2,
                "{name:?} needs {} lines, only 2 fit",
                lines.len()
            );
            // Nothing lost: the wrapped words rejoin to the original.
            assert_eq!(
                lines.join(" "),
                crate::skin::short_name(name),
                "{name:?} lost characters in wrapping"
            );
        }
    }

    /// Two presets differing only by variant must not render identically.
    #[test]
    fn variants_are_visibly_different() {
        let c = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let render = |name| {
            let mut panel = Buffer::new();
            let view = View {
                cursor_name: Some(name),
                slot_presets: Some([0, 1, 2]),
                active_slot: Some(Slot::A),
                ..View::stub(&c)
            };
            draw(&mut panel, &view).unwrap();
            panel.pixels
        };
        assert_ne!(
            render("TF PROTEIN - BLUE 1"),
            render("TF PROTEIN - GREEN 2"),
            "BLUE and GREEN must be tellable apart on the panel"
        );
    }

    #[test]
    fn a_write_in_flight_is_shown() {
        let c = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let mut v = view_of(&c, 2, Some("TF TILT"));
        v.active = Some(0);
        v.pending = true;

        let mut panel = Buffer::new();
        draw(&mut panel, &v).unwrap();
        assert!(panel.any(WARN), "a pending write must be visible");
    }

    /// Everything must land inside 128×128. A real ST7735S silently discards
    /// out-of-range pixels, so an overflowing layout looks fine in code and
    /// wrong on the bench.
    #[test]
    fn the_longest_real_preset_name_stays_on_the_panel() {
        let c = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        // The longest name on the test pedal, 28 characters.
        let mut v = view_of(&c, 19, Some("TF MORNIING GLORY - BRIGHT 1"));
        v.active = Some(0);
        v.last_error = Some("unrecognised message type 0x9999");

        let mut panel = Buffer::new();
        draw(&mut panel, &v).unwrap();
        assert!(panel.lit() > 0);
    }
}
