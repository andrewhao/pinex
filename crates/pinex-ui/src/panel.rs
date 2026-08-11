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

use embedded_graphics::mono_font::ascii::{FONT_10X20, FONT_6X10, FONT_9X15_BOLD};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{CornerRadii, PrimitiveStyle, Rectangle, RoundedRectangle};
use embedded_graphics::text::{Alignment, Baseline, Text};

use crate::browser::{Connection, View};

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
        Connection::Connected { firmware } => draw_browser(target, view, firmware),
    }
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

fn draw_browser<D>(target: &mut D, view: &View<'_>, firmware: &str) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let small = MonoTextStyle::new(&FONT_6X10, DIM);
    let name_style = MonoTextStyle::new(&FONT_6X10, TEXT);

    // Header: firmware, right-aligned so the number below owns the centre.
    Text::with_baseline(
        &format!("fw {firmware}"),
        Point::new(2, 1),
        small,
        Baseline::Top,
    )
    .draw(target)?;

    // The pedal's own colour for the browsed preset, as a bar down the side —
    // so the panel and the pedal's ring agree at a glance.
    if let Some([r, g, b]) = view.cursor_color {
        RoundedRectangle::new(
            Rectangle::new(Point::new(118, 2), Size::new(8, 10)),
            CornerRadii::new(Size::new(2, 2)),
        )
        // Rgb565 takes 5/6/5-bit channels, so the pedal's 8-bit values are
        // narrowed rather than passed through.
        .into_styled(PrimitiveStyle::with_fill(Rgb565::new(
            r >> 3,
            g >> 2,
            b >> 3,
        )))
        .draw(target)?;
    }

    // The number, as large as the panel allows. Green when this is what is
    // playing, white when the player is browsing elsewhere.
    let browsing_elsewhere = view.active != Some(view.cursor);
    let number_color = if browsing_elsewhere { TEXT } else { PLAYING };
    let number = MonoTextStyle::new(&FONT_10X20, number_color);

    Text::with_alignment(
        &format!("{:02}", view.cursor + 1),
        Point::new(WIDTH as i32 / 2, 44),
        number,
        Alignment::Center,
    )
    .draw(target)?;

    // Name, wrapped over two lines.
    let name = view.cursor_name.unwrap_or("...");
    for (line, text) in wrap(name, NAME_COLS).iter().take(2).enumerate() {
        Text::with_alignment(
            text,
            Point::new(WIDTH as i32 / 2, 60 + (line as i32 * 11)),
            name_style,
            Alignment::Center,
        )
        .draw(target)?;
    }

    // Footer: what is actually playing, whenever that differs from the cursor.
    // This is the line that stops the panel implying a preset is loaded when it
    // is only being looked at.
    let footer = MonoTextStyle::new(&FONT_9X15_BOLD, PLAYING);
    if view.pending {
        Text::with_alignment(
            "sending...",
            Point::new(WIDTH as i32 / 2, 104),
            MonoTextStyle::new(&FONT_9X15_BOLD, WARN),
            Alignment::Center,
        )
        .draw(target)?;
    } else if browsing_elsewhere {
        let playing = match view.active {
            Some(active) => format!("now {:02}", active + 1),
            None => "now --".to_string(),
        };
        Text::with_alignment(
            &playing,
            Point::new(WIDTH as i32 / 2, 104),
            footer,
            Alignment::Center,
        )
        .draw(target)?;
    }

    if let Some(error) = view.last_error {
        let err = MonoTextStyle::new(&FONT_6X10, WARN);
        let short: String = error.chars().take(NAME_COLS).collect();
        Text::with_alignment(
            &short,
            Point::new(WIDTH as i32 / 2, 120),
            err,
            Alignment::Center,
        )
        .draw(target)?;
    }
    Ok(())
}

/// Break `text` into lines of at most `cols` characters, on word boundaries
/// where it can and mid-word where a single word is too long.
fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        if word.len() > cols {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            for chunk in word.as_bytes().chunks(cols) {
                lines.push(String::from_utf8_lossy(chunk).into_owned());
            }
            continue;
        }
        let would_be = if current.is_empty() {
            word.len()
        } else {
            current.len() + 1 + word.len()
        };
        if would_be > cols {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
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
            connection,
            cursor,
            cursor_name: name,
            cursor_color: None,
            active: Some(cursor),
            active_name: name,
            pending: false,
            last_error: None,
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

    #[test]
    fn the_playing_preset_is_drawn_in_the_playing_colour() {
        let mut panel = Buffer::new();
        let c = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        draw(&mut panel, &view_of(&c, 4, Some("TF TILT - 1 ADV"))).unwrap();
        assert!(
            panel.any(PLAYING),
            "cursor on the active preset reads green"
        );
    }

    /// Browsing away from what is playing must be visually distinct, or the
    /// panel implies a preset is loaded when it is only being looked at.
    #[test]
    fn browsing_away_from_the_active_preset_looks_different() {
        let c = Connection::Connected {
            firmware: "1.3.17".into(),
        };

        let mut on = Buffer::new();
        draw(&mut on, &view_of(&c, 4, Some("TF TILT"))).unwrap();

        let mut away = Buffer::new();
        let mut v = view_of(&c, 4, Some("TF TILT"));
        v.active = Some(9);
        draw(&mut away, &v).unwrap();

        assert_ne!(
            on.pixels, away.pixels,
            "the two states must not render identically"
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

    #[test]
    fn names_wrap_on_word_boundaries_and_split_only_when_forced() {
        assert_eq!(wrap("TF TILT - 1 ADV", 21), vec!["TF TILT - 1 ADV"]);
        assert_eq!(
            wrap("TF MORNIING GLORY - BRIGHT 1", 21),
            vec!["TF MORNIING GLORY -", "BRIGHT 1"]
        );
        // A single word longer than the line is cut, not lost.
        assert_eq!(wrap("ABCDEFGHIJ", 4), vec!["ABCD", "EFGH", "IJ"]);
        assert!(wrap("", 10).is_empty());
    }
}
