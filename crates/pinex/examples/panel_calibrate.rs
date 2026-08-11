//! Draw a calibration pattern so panel alignment can be judged by eye.
//!
//! ```sh
//! PINEX_PANEL_ROTATION=90 PINEX_PANEL_OFFSET=2,1 \
//!   ~/bin/panel_calibrate
//! ```
//!
//! What to look for:
//!
//! - **The white border must touch all four edges** with no gap and no band of
//!   noise outside it. A gap means the offset is too large; noise means too
//!   small, because the window then includes controller RAM we never wrote.
//! - **"TOP LEFT" must be in the top-left**, reading normally. If it is
//!   elsewhere, change the rotation.
//! - **The red/green/blue bars must read R, G, B left to right.** If red and
//!   blue are swapped, the colour order is BGR rather than RGB.

use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;

use pinex_ui::hat::HatDisplay;
use pinex_ui::panel::{HEIGHT, WIDTH};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut display = HatDisplay::open()?;
    println!(
        "rotation={} offset={}",
        std::env::var("PINEX_PANEL_ROTATION").unwrap_or_else(|_| "90 (default)".into()),
        std::env::var("PINEX_PANEL_OFFSET").unwrap_or_else(|_| "derived from rotation".into())
    );

    display.with_target(|target| -> Result<(), core::convert::Infallible> {
        target.clear(Rgb565::BLACK).ok();

        // Border on the very outermost pixels: the whole point is that it
        // should touch every edge.
        Rectangle::new(Point::zero(), Size::new(WIDTH, HEIGHT))
            .into_styled(PrimitiveStyle::with_stroke(Rgb565::WHITE, 1))
            .draw(target)
            .ok();

        // Corner markers, so a rotation is obvious rather than plausible.
        let label = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);
        Text::new("TOP LEFT", Point::new(4, 12), label)
            .draw(target)
            .ok();
        Text::new(
            "BR",
            Point::new(WIDTH as i32 - 18, HEIGHT as i32 - 4),
            label,
        )
        .draw(target)
        .ok();

        // Primary bars: if red and blue swap, the colour order is BGR.
        for (index, color) in [Rgb565::RED, Rgb565::GREEN, Rgb565::BLUE]
            .into_iter()
            .enumerate()
        {
            Rectangle::new(Point::new(10 + index as i32 * 36, 50), Size::new(30, 24))
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(target)
                .ok();
        }

        Text::new("R    G    B", Point::new(14, 88), label)
            .draw(target)
            .ok();
        Ok(())
    })?;

    println!("pattern drawn — leaving it up for 60s");
    std::thread::sleep(std::time::Duration::from_secs(60));
    Ok(())
}
