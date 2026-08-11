//! Raw rppal read of the HAT inputs. Diagnostic only.
//!
//! `pinctrl` sees presses on all eight pins; the app did not. This prints what
//! rppal reads, with and without the display opened first, to find where the
//! signal is lost.

use std::time::{Duration, Instant};

use pinex_input::hat::{HatButtons, LABELS};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let hold_display = std::env::args().any(|a| a == "--with-display");

    // Kept alive if requested: the app opens the display before the buttons,
    // and two rppal Gpio handles may not coexist as assumed.
    let _display = if hold_display {
        eprintln!("opening the display first, as the app does");
        Some(pinex_ui::hat::HatDisplay::open()?)
    } else {
        eprintln!("buttons only, no display");
        None
    };

    let mut buttons = HatButtons::open()?;
    // The same samples, run through the same debouncer the app uses, so one
    // press tells us whether rppal saw it AND whether the debounce passed it.
    let mut debounce: pinex_input::debounce::Debouncer<8> = Default::default();

    println!("press any button a few times; 25s");
    println!("{:>6}  RAW       DEBOUNCED", "t");

    let started = Instant::now();
    let mut last = String::new();
    let mut raw_edges = 0usize;
    let mut fired = 0usize;

    while started.elapsed() < Duration::from_secs(25) {
        let levels = buttons.raw_levels();
        let line: String = levels.iter().map(|l| if *l { '1' } else { '0' }).collect();

        let pressed = debounce.sample(levels);
        for index in &pressed {
            fired += 1;
            println!(
                "{:>6.1}  {line}  FIRED {}",
                started.elapsed().as_secs_f32(),
                LABELS[*index]
            );
        }
        if line != last {
            if line.contains('1') {
                raw_edges += 1;
            }
            if pressed.is_empty() {
                println!("{:>6.1}  {line}", started.elapsed().as_secs_f32());
            }
            last = line;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    println!("done: {raw_edges} raw press edges, {fired} debounced events");
    Ok(())
}
