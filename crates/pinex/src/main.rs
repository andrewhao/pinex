//! Pinex — a preset browser for the IK Multimedia Tonex ONE.
//!
//! ```sh
//! cargo run -p pinex                       # find the pedal automatically
//! cargo run -p pinex -- /dev/cu.usbmodem1  # or name it
//! cargo run -p pinex -- --sim              # no pedal: run against the simulator
//! ```
//!
//! Keys: `n`/Enter next, `p` previous, `s` select, `r` refresh, `q` quit.

use std::path::PathBuf;

use pinex::App;
use pinex_device::Pedal;
use pinex_input::StdinInput;
use pinex_ui::ConsoleRenderer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1);

    // Held for the lifetime of the run: dropping it stops the simulated pedal.
    let mut _sim = None;

    let device = match arg.as_deref() {
        Some("--sim") => {
            let sim = pinex_device::sim::PedalSim::start()?;
            let path = sim.device_path().to_path_buf();
            eprintln!("simulated pedal on {}", path.display());
            _sim = Some(sim);
            path
        }
        Some("--help" | "-h") => {
            eprintln!("{}", USAGE);
            return Ok(());
        }
        Some(path) => PathBuf::from(path),
        None => find_pedal().ok_or(
            "no Tonex found. Plug it in, pass the tty path, or use --sim to run without one.",
        )?,
    };

    eprintln!("opening {}", device.display());
    let pedal = Pedal::open(&device)?;
    let mut app = App::new(pedal, StdinInput::new(), ConsoleRenderer);

    app.start()?;
    eprintln!("{}", USAGE);

    while app.step() {
        for error in app.errors.drain(..) {
            eprintln!("! {error}");
        }
    }
    Ok(())
}

const USAGE: &str = "keys: n/Enter = next, p = prev, s = select, r = refresh, q = quit";

/// Find the pedal's tty.
///
/// The udev rule in `deploy/` gives it a stable `/dev/tonex` on Linux. Failing
/// that, fall back to the single CDC-ACM node it enumerates as. This is a
/// convenience, not a guarantee — pass the path explicitly if it guesses wrong.
fn find_pedal() -> Option<PathBuf> {
    for stable in ["/dev/tonex", "/dev/ttyACM0"] {
        let path = PathBuf::from(stable);
        if path.exists() {
            return Some(path);
        }
    }

    // macOS: the callout device, which does not wait for carrier detect.
    let mut candidates: Vec<PathBuf> = std::fs::read_dir("/dev")
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("cu.usbmodem"))
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}
