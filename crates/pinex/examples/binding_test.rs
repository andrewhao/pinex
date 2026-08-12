//! Check the documented bindings against what the hardware actually does.
//!
//! For every physical press this prints the whole chain: which input fired,
//! which `InputEvent` it resolved to, what the browser did with it, and where
//! that left the page, cursor and selected slot.
//!
//! Compare directly against the table in `docs/manual.md`. It drives a
//! simulated pedal so the state is realistic without touching the real one.

use std::time::{Duration, Instant};

use pinex_device::sim::PedalSim;
use pinex_device::{Pedal, PedalEvent};
use pinex_input::hat::HatButtons;
use pinex_input::InputSource;
use pinex_input::{BINDINGS, LABELS};
use pinex_ui::PresetBrowser;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("documented bindings, from docs/manual.md:");
    for (label, event) in LABELS.iter().zip(BINDINGS.iter()) {
        println!("  {label:<10} -> {event:?}");
    }
    println!("\npress each control; 10 minutes\n");

    // A simulated pedal, so the browser has realistic state to act on without
    // risking the real one while bindings are being checked.
    let sim = PedalSim::start()?;
    let mut pedal = Pedal::open(sim.device_path())?;
    let mut browser = PresetBrowser::new();
    pedal.hello()?;

    let mut buttons = HatButtons::open()?;
    let started = Instant::now();
    let mut count = 0usize;

    while started.elapsed() < Duration::from_secs(600) {
        // Keep the browser in step with the pedal so pages behave normally.
        while let Ok(event) = pedal.next_event(Duration::from_millis(0)) {
            let _ = browser.apply(&event);
            if let PedalEvent::Disconnected = event {
                break;
            }
        }

        if let Some(index) = buttons.poll_raw(Duration::from_millis(50)) {
            count += 1;
            let event = BINDINGS[index];
            let before = {
                let v = browser.view();
                (v.screen, v.cursor, v.selected)
            };
            let commands = browser.handle(event);
            let after = {
                let v = browser.view();
                (v.screen, v.cursor, v.selected)
            };

            println!("{count:>3}. {:<10} -> {:?}", LABELS[index], event);
            println!(
                "      page {:?}->{:?}  cursor {}->{}  editing {:?}->{:?}",
                before.0, after.0, before.1, after.1, before.2, after.2
            );
            if commands.is_empty() {
                println!("      (no command sent)");
            } else {
                println!("      commands: {commands:?}");
            }
        }
    }
    println!("\n{count} presses seen");
    Ok(())
}
