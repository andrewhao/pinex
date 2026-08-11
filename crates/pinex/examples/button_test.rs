//! Check every HAT input registers a press — on the panel, so one person can
//! do it alone.
//!
//! ```sh
//! ~/bin/button_test          # 120s, or until all eight are seen
//! ```
//!
//! Press each input once. The panel ticks it off and the console logs it, so
//! the result is readable both by the person pressing and by whoever reads the
//! log afterwards.
//!
//! This verifies the half the debounce tests cannot: those prove a noisy input
//! never fires, which is worthless if a real press does not fire either.

use std::time::{Duration, Instant};

use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_9X15_BOLD};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::text::{Alignment, Text};

use pinex_input::hat::{HatButtons, LABELS};
use pinex_ui::hat::HatDisplay;
use pinex_ui::panel::WIDTH;

/// Generous on purpose. An earlier version ran for two minutes from launch and
/// expired before anyone reached the buttons; its "0 presses" then looked like
/// a code fault rather than an empty room. The window should never be the thing
/// under test.
const RUN_FOR: Duration = Duration::from_secs(900);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut display = HatDisplay::open()?;
    let mut buttons = HatButtons::open()?;

    let mut seen = [false; 8];
    let mut presses = 0usize;
    // The clock starts at the *first* press, not at launch, so however long it
    // takes someone to get to the HAT is not counted against them.
    let launched = Instant::now();
    let mut started: Option<Instant> = None;

    println!(
        "press each of the eight inputs once; {}s",
        RUN_FOR.as_secs()
    );
    draw(&mut display, &seen, None);

    while launched.elapsed() < RUN_FOR {
        if let Some(index) = buttons.poll_raw(Duration::from_millis(50)) {
            presses += 1;
            seen[index] = true;
            let clock = *started.get_or_insert_with(Instant::now);
            println!(
                "[{:>5.1}s] {} (index {index})  {}/8 seen",
                clock.elapsed().as_secs_f32(),
                LABELS[index],
                seen.iter().filter(|s| **s).count()
            );
            draw(&mut display, &seen, Some(index));

            if seen.iter().all(|s| *s) {
                println!("ALL EIGHT INPUTS REGISTERED ({presses} presses total)");
                draw(&mut display, &seen, None);
                std::thread::sleep(Duration::from_secs(5));
                return Ok(());
            }
        }
    }

    let missing: Vec<&str> = LABELS
        .iter()
        .zip(seen.iter())
        .filter(|(_, s)| !**s)
        .map(|(l, _)| *l)
        .collect();
    if presses == 0 {
        println!(
            "NO PRESSES AT ALL in {}s — the window expired with nobody at the HAT, \
             which is not the same as the buttons being broken.",
            RUN_FOR.as_secs()
        );
    }
    println!("TIMED OUT — never saw: {missing:?} ({presses} presses total)");
    Ok(())
}

/// A tick list, so the person pressing can see what is left.
fn draw(display: &mut HatDisplay, seen: &[bool; 8], last: Option<usize>) {
    let done = seen.iter().filter(|s| **s).count();
    let _ = display.with_target(|target| -> Result<(), core::convert::Infallible> {
        target.clear(Rgb565::BLACK).ok();

        let heading = MonoTextStyle::new(&FONT_9X15_BOLD, Rgb565::WHITE);
        Text::with_alignment(
            &format!("{done}/8"),
            Point::new(WIDTH as i32 / 2, 13),
            heading,
            Alignment::Center,
        )
        .draw(target)
        .ok();

        for (index, label) in LABELS.iter().enumerate() {
            let color = if Some(index) == last {
                Rgb565::CSS_ORANGE
            } else if seen[index] {
                Rgb565::CSS_LIME_GREEN
            } else {
                Rgb565::CSS_DIM_GRAY
            };
            let mark = if seen[index] { "x" } else { " " };
            Text::new(
                &format!("[{mark}] {label}"),
                Point::new(8, 32 + index as i32 * 12),
                MonoTextStyle::new(&FONT_6X10, color),
            )
            .draw(target)
            .ok();
        }
        Ok(())
    });
}
