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
        // No pedal yet is not an error: wait for one.
        None => find_pedal().unwrap_or_else(|| PathBuf::from("/dev/tonex")),
    };

    eprintln!("watching {}", device.display());
    // Reconnecting rather than opening once: the pedal may be plugged in after
    // the service starts, or unplugged mid-set. Neither should end the program —
    // the display says NO PEDAL and the loop keeps going.
    let mut app = App::reconnecting(device, StdinInput::new(), ConsoleRenderer);

    // The debug page is how a firmware change gets discovered, so it runs by
    // default. Port 0 or a bind failure must not stop the pedal working.
    let web = pinex_web::DebugServer::start(web_port(), app.snapshot());
    match &web {
        Ok(server) => eprintln!("debug page: http://localhost:{}/", server.addr().port()),
        Err(e) => eprintln!("! debug page unavailable: {e}"),
    }

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

/// Debug page port. `PINEX_WEB_PORT=0` picks a free one; unset uses 8080.
fn web_port() -> u16 {
    std::env::var("PINEX_WEB_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080)
}

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
