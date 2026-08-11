//! Seeing the panel without the panel.
//!
//! [`PreviewPanel`] is a `DrawTarget` that keeps a 128×128 framebuffer and can
//! print it to a terminal in truecolour. The same [`crate::panel::draw`] that
//! drives the ST7735S renders into it, so what you see locally is what the
//! glass shows — not an approximation of it.
//!
//! Two pixels per character cell, using the upper-half block `▀`: foreground is
//! the top pixel, background the bottom. A 128×128 panel is then 128 columns by
//! 64 rows, which fits an ordinary terminal.
//!
//! This exists because iterating on the screen against real hardware means a
//! cross-compile, a copy, a restart and someone physically looking at it. That
//! loop is far too slow for layout work, and it needs a person in the room.

use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::Rectangle;

use crate::browser::View;
use crate::panel;
use crate::render::Renderer;

/// An in-memory panel that can draw itself as text.
#[derive(Clone)]
pub struct PreviewPanel {
    pixels: Vec<Rgb565>,
    width: u32,
    height: u32,
}

impl Default for PreviewPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewPanel {
    pub fn new() -> Self {
        Self::with_size(panel::WIDTH, panel::HEIGHT)
    }

    pub fn with_size(width: u32, height: u32) -> Self {
        Self {
            pixels: vec![Rgb565::BLACK; (width * height) as usize],
            width,
            height,
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> Rgb565 {
        self.pixels[(y * self.width + x) as usize]
    }

    /// How many pixels are not the background. A quick "did anything draw?".
    pub fn lit(&self) -> usize {
        self.pixels.iter().filter(|p| **p != Rgb565::BLACK).count()
    }

    /// Render as ANSI truecolour, two pixels per character cell.
    ///
    /// `scale` of 1 shows every pixel; 2 samples every other one, halving both
    /// dimensions for a narrow terminal.
    pub fn to_ansi(&self, scale: u32) -> String {
        let scale = scale.max(1);
        let mut out = String::new();

        let mut y = 0;
        while y < self.height {
            let mut x = 0;
            while x < self.width {
                let top = self.pixel(x, y);
                // The bottom pixel of the cell, or background past the edge.
                let bottom = if y + scale < self.height {
                    self.pixel(x, y + scale)
                } else {
                    Rgb565::BLACK
                };
                out.push_str(&format!(
                    "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m▀",
                    r8(top),
                    g8(top),
                    b8(top),
                    r8(bottom),
                    g8(bottom),
                    b8(bottom)
                ));
                x += scale;
            }
            out.push_str("\x1b[0m\n");
            y += scale * 2;
        }
        out
    }

    /// Render as plain ASCII, one character per pixel-pair, by brightness.
    ///
    /// Colourless and diffable, which is what makes it useful in a test: a
    /// layout regression shows up as a readable picture rather than as a pixel
    /// count that moved from 1,482 to 1,477.
    pub fn to_ascii(&self, scale: u32) -> String {
        const RAMP: [char; 5] = [' ', '.', ':', '*', '#'];
        let scale = scale.max(1);
        let mut out = String::new();

        let mut y = 0;
        while y < self.height {
            let mut x = 0;
            while x < self.width {
                let top = luminance(self.pixel(x, y));
                let bottom = if y + scale < self.height {
                    luminance(self.pixel(x, y + scale))
                } else {
                    0
                };
                let level = (top.max(bottom) as usize * (RAMP.len() - 1)) / 255;
                out.push(RAMP[level]);
                x += scale;
            }
            out.push('\n');
            y += scale * 2;
        }
        out
    }

    /// A border of `-` and `|` around the panel, so the edges are visible
    /// against a dark terminal — misalignment at an edge is otherwise invisible.
    pub fn to_ansi_framed(&self, scale: u32) -> String {
        let cols = (self.width / scale.max(1)) as usize;
        let mut out = String::with_capacity(cols * 80);
        out.push('+');
        out.push_str(&"-".repeat(cols));
        out.push_str("+\n");
        for line in self.to_ansi(scale).lines() {
            out.push('|');
            out.push_str(line);
            out.push_str("|\n");
        }
        out.push('+');
        out.push_str(&"-".repeat(cols));
        out.push_str("+\n");
        out
    }
}

/// Rough perceptual brightness, for the ASCII ramp.
fn luminance(c: Rgb565) -> u8 {
    let (r, g, b) = (r8(c) as u32, g8(c) as u32, b8(c) as u32);
    ((r * 30 + g * 59 + b * 11) / 100).min(255) as u8
}

fn r8(c: Rgb565) -> u8 {
    // Rgb565's 5-bit channel scaled back to 8 bits.
    (c.r() as u16 * 255 / 31) as u8
}
fn g8(c: Rgb565) -> u8 {
    (c.g() as u16 * 255 / 63) as u8
}
fn b8(c: Rgb565) -> u8 {
    (c.b() as u16 * 255 / 31) as u8
}

impl Dimensions for PreviewPanel {
    fn bounding_box(&self) -> Rectangle {
        Rectangle::new(Point::zero(), Size::new(self.width, self.height))
    }
}

impl DrawTarget for PreviewPanel {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            // Off-panel writes are dropped, as the real controller drops them.
            // Silently — which is exactly why the layout tests check bounds.
            if (0..self.width as i32).contains(&point.x)
                && (0..self.height as i32).contains(&point.y)
            {
                let index = point.y as usize * self.width as usize + point.x as usize;
                self.pixels[index] = color;
            }
        }
        Ok(())
    }
}

/// Draws the panel to the terminal on every change.
///
/// Slots into the app loop exactly where [`crate::hat::HatDisplay`] would on the
/// Pi, so the harness exercises the real render path rather than a mock of it.
pub struct PreviewRenderer {
    panel: PreviewPanel,
    scale: u32,
    /// Redraw in place rather than scrolling, when the terminal allows it.
    in_place: bool,
}

impl Default for PreviewRenderer {
    fn default() -> Self {
        Self::new(1)
    }
}

impl PreviewRenderer {
    pub fn new(scale: u32) -> Self {
        Self {
            panel: PreviewPanel::new(),
            scale,
            in_place: true,
        }
    }

    /// Scroll each frame instead of redrawing in place. Useful when piping.
    pub fn scrolling(mut self) -> Self {
        self.in_place = false;
        self
    }

    pub fn panel(&self) -> &PreviewPanel {
        &self.panel
    }
}

impl Renderer for PreviewRenderer {
    fn render(&mut self, view: &View<'_>) {
        // Infallible target, so this cannot fail.
        let _ = panel::draw(&mut self.panel, view);
        if self.in_place {
            // Home the cursor and clear below, so the panel stays put.
            print!("\x1b[H\x1b[J");
        }
        print!("{}", self.panel.to_ansi_framed(self.scale));
        println!("{}", crate::render::lines(view).join("  |  "));
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::Connection;

    fn view_of(connection: &Connection) -> View<'_> {
        View {
            connection,
            cursor: 0,
            cursor_name: Some("TF BENSON PREAMP - 1"),
            cursor_color: Some([255, 63, 0]),
            active: Some(0),
            active_name: Some("TF BENSON PREAMP - 1"),
            pending: false,
            last_error: None,
        }
    }

    #[test]
    fn the_preview_renders_the_same_draw_call_the_hardware_uses() {
        let connected = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let mut panel = PreviewPanel::new();
        panel::draw(&mut panel, &view_of(&connected)).unwrap();
        assert!(panel.lit() > 0, "something must have been drawn");
    }

    #[test]
    fn ansi_output_has_one_row_per_two_pixel_rows() {
        let panel = PreviewPanel::new();
        let rows = panel.to_ansi(1).lines().count();
        assert_eq!(rows, (panel::HEIGHT / 2) as usize);

        // Scaling halves both dimensions.
        let scaled = panel.to_ansi(2).lines().count();
        assert_eq!(scaled, (panel::HEIGHT / 4) as usize);
    }

    #[test]
    fn colours_survive_the_round_trip_to_ansi() {
        let mut panel = PreviewPanel::new();
        panel
            .draw_iter([Pixel(Point::new(0, 0), Rgb565::RED)])
            .unwrap();
        // Full-scale red is 255,0,0 after the 5-bit to 8-bit expansion.
        assert!(
            panel.to_ansi(1).contains("38;2;255;0;0"),
            "red should reach the terminal as full red"
        );
    }

    /// A readable snapshot of the screen the spec insists on. If a layout
    /// change moves this, the diff shows what it now looks like.
    #[test]
    fn the_no_pedal_screen_looks_like_this() {
        let disconnected = Connection::Disconnected;
        let mut panel = PreviewPanel::new();
        panel::draw(&mut panel, &view_of(&disconnected)).unwrap();

        let art = panel.to_ascii(4);
        // Two words, centred, in the middle third of the panel.
        let rows: Vec<&str> = art.lines().collect();
        assert_eq!(rows.len(), 16, "128px at scale 4 is 16 rows");

        // Any non-space is ink. Thresholding on the brighter ramp characters
        // would miss the warning colour, whose luminance is middling.
        let inked: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.trim().chars().any(|c| c != ' '))
            .map(|(i, _)| i)
            .collect();
        assert!(
            inked.iter().all(|row| (5..=10).contains(row)),
            "NO PEDAL should sit in the middle of the panel, drew on rows {inked:?}"
        );
        assert!(!inked.is_empty(), "something must be drawn");
    }

    /// The frame is what makes an edge-alignment problem visible; without it a
    /// dark panel and a dark terminal are indistinguishable.
    #[test]
    fn the_framed_output_is_bounded_by_a_visible_border() {
        let panel = PreviewPanel::with_size(8, 8);
        let framed = panel.to_ansi_framed(1);
        let lines: Vec<&str> = framed.lines().collect();

        assert_eq!(lines.first(), Some(&"+--------+"));
        assert_eq!(lines.last(), Some(&"+--------+"));
        for line in &lines[1..lines.len() - 1] {
            assert!(line.starts_with('|') && line.ends_with('|'));
        }
    }
}
