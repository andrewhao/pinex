//! Live verification of the write path against real hardware.
//!
//! This is the one thing the simulator cannot establish: that the pedal
//! *accepts* what we send. The simulator recognises our writes because they are
//! ours, so it can never be surprised by them.
//!
//! ```sh
//! cargo run -p pinex --example verify_write -- /dev/cu.usbmodem201134301
//! ```
//!
//! It waits for the pedal to appear and answer, so it can be started before the
//! pedal is plugged in and will complete on its own once it is.
//!
//! **It puts the pedal back where it found it.** The original preset is restored
//! before exit, including on failure, so running this never leaves someone's rig
//! on the wrong sound.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pinex_device::{Pedal, PedalEvent};
use pinex_proto::message;
use pinex_proto::state::PedalState;

const REPLY_TIMEOUT: Duration = Duration::from_secs(3);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/cu.usbmodem201134301"));
    let wait: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);

    println!("waiting up to {wait}s for a pedal on {}", path.display());
    let (mut pedal, firmware) = wait_for_pedal(&path, Duration::from_secs(wait))?;
    println!("PEDAL RESPONDING — firmware {firmware}");

    let original = fetch_state(&mut pedal)?;
    let original_preset = original.active_preset()?;
    let original_slot = original.active_slot()?;
    println!(
        "baseline: preset {} (slot {original_slot:?})",
        original_preset + 1
    );

    // Pick a target that is definitely a change.
    let target = if original_preset == 0 { 1 } else { 0 };
    println!("--> switching to preset {}", target + 1);

    let result = attempt_switch(&mut pedal, &original, target);

    // Restore first, report second: the pedal must go back regardless.
    // Give the pedal a moment to settle before reading state to restore from.
    std::thread::sleep(Duration::from_millis(800));
    println!("<-- restoring preset {}", original_preset + 1);
    let restored = match fetch_state(&mut pedal) {
        Ok(current) => match message::set_preset(&current, original_preset) {
            Ok((frame, _)) => {
                pedal.send_frame(&frame)?;
                std::thread::sleep(Duration::from_millis(1500));
                matches!(fetch_state(&mut pedal), Ok(s) if s.active_preset() == Ok(original_preset))
            }
            Err(e) => {
                eprintln!("!! could not build the restore frame: {e}");
                false
            }
        },
        Err(e) => {
            eprintln!("!! could not read state to restore: {e}");
            false
        }
    };

    match result {
        Ok(observed) => {
            println!();
            println!(
                "RESULT: PASS — the pedal accepted the write and reports preset {}",
                observed + 1
            );
            println!(
                "restored to original: {}",
                if restored {
                    "yes"
                } else {
                    "NO — CHECK THE PEDAL"
                }
            );
            Ok(())
        }
        Err(e) => {
            println!();
            println!("RESULT: FAIL — {e}");
            println!(
                "restored to original: {}",
                if restored {
                    "yes"
                } else {
                    "NO — CHECK THE PEDAL"
                }
            );
            Err(e)
        }
    }
}

/// Poll until a pedal answers Hello, so this can be started before it is plugged in.
fn wait_for_pedal(
    path: &Path,
    budget: Duration,
) -> Result<(Pedal, String), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + budget;
    let mut announced = false;

    while Instant::now() < deadline {
        if !path.exists() {
            if !announced {
                println!("  (no device node yet)");
                announced = true;
            }
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }

        if let Ok(mut pedal) = Pedal::open(path) {
            if pedal.hello().is_ok() {
                if let Ok(PedalEvent::Connected { firmware }) = pedal.next_event(REPLY_TIMEOUT) {
                    return Ok((pedal, firmware));
                }
            }
        }
        print!(".");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        std::thread::sleep(Duration::from_secs(2));
    }
    Err("pedal never answered".into())
}

fn fetch_state(pedal: &mut Pedal) -> Result<PedalState, Box<dyn std::error::Error>> {
    pedal.request_state()?;
    await_state(pedal)
}

/// Wait for the next state, ignoring other traffic.
fn await_state(pedal: &mut Pedal) -> Result<PedalState, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + REPLY_TIMEOUT;
    while Instant::now() < deadline {
        match pedal.next_event(Duration::from_millis(250)) {
            Ok(PedalEvent::StateChanged(state)) => return Ok(state),
            Ok(PedalEvent::ParseError { reason, raw }) => {
                eprintln!("!! parse error: {reason}\n   raw: {}", pinex_web::hex(&raw));
            }
            Ok(_) => continue,
            Err(_) => continue,
        }
    }
    Err("no state response".into())
}

/// Send the verified write and confirm the pedal reports the new preset.
fn attempt_switch(
    pedal: &mut Pedal,
    from: &PedalState,
    target: u8,
) -> Result<u8, Box<dyn std::error::Error>> {
    let (frame, touched) = message::set_preset(from, target)?;
    println!(
        "    write verified locally: touches offsets {touched:?} of {} bytes",
        from.len()
    );

    pedal.send_frame(&frame)?;
    let echoed = await_state(pedal)?;

    // Do NOT trust the first reply. The pedal accepts a write, echoes the new
    // state, and can revert it about a second later — an earlier version of
    // this harness returned here and reported a PASS for a change that did not
    // stick. Let it settle, then ask again.
    std::thread::sleep(Duration::from_millis(1500));
    let settled = fetch_state(pedal)?;
    let observed = settled.active_preset()?;

    if observed != target {
        return Err(format!(
            "asked for preset {}; pedal echoed {} then settled on {} — the write did not stick",
            target + 1,
            echoed.active_preset()? + 1,
            observed + 1
        )
        .into());
    }

    // The pedal must not have been resized or mangled by our write.
    if settled.len() != from.len() {
        return Err(format!(
            "state changed length: sent {} bytes, got {} back",
            from.len(),
            settled.len()
        )
        .into());
    }

    // Direct monitoring must still be on, or the pedal is silent over USB.
    if settled.direct_monitoring() != 1 {
        return Err("direct monitoring is off after the write — the pedal would be muted".into());
    }

    Ok(observed)
}
