//! The whole thing, on your desk, with no hardware at all.
//!
//! ```sh
//! cargo run -p pinex --example panel_sim          # simulated pedal
//! cargo run -p pinex --example panel_sim -- /dev/cu.usbmodem…   # a real one
//! ```
//!
//! Runs the real app loop — real framing, real reader thread, real browser —
//! against the PTY pedal simulator, and draws the panel into the terminal in
//! truecolour. Keys are the usual `n`/`p`/`s`/`r`/`q`.
//!
//! This is the loop to iterate the screen in. Changing a layout and seeing the
//! result costs a `cargo run`, not a cross-compile, a copy, a service restart
//! and someone in the room to look at the glass.
//!
//! What it cannot tell you: the real panel's orientation, its window offset, or
//! its colour order. Those are properties of the glass, and `panel_calibrate`
//! on the Pi is what settles them.

use std::path::PathBuf;

use pinex::App;
use pinex_input::StdinInput;
use pinex_ui::{Multi, PreviewRenderer};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scale: u32 = std::env::var("PINEX_PREVIEW_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);

    // Held for the whole run: dropping it stops the simulated pedal.
    let mut _sim = None;
    let device = match std::env::args().nth(1) {
        Some(path) => PathBuf::from(path),
        None => {
            let sim = pinex_device::sim::PedalSim::start()?;
            let path = sim.device_path().to_path_buf();
            _sim = Some(sim);
            path
        }
    };

    eprintln!("panel preview — keys: n/p = browse, s = select, r = refresh, q = quit");
    eprintln!("PINEX_PREVIEW_SCALE=2 for a narrower terminal");

    // Preview only: a second renderer printing below would scroll the panel
    // out of place on every frame.
    let renderers = Multi::default().with(PreviewRenderer::new(scale));

    let mut app = App::reconnecting(device, StdinInput::new(), renderers);
    app.start()?;

    while app.step() {
        for error in app.errors.drain(..) {
            eprintln!("! {error}");
        }
    }
    Ok(())
}
