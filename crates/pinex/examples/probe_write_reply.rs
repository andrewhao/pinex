//! Capture exactly what the pedal sends back after a state write.
//!
//! Diagnostic only. `pinex` saw an unrecognised message type `0x0005` following
//! a preset change, and guessing at it would be exactly the mistake this project
//! keeps refusing to make.
//!
//! Restores the original preset before exiting.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pinex_device::{Pedal, PedalEvent};
use pinex_proto::message;
use pinex_proto::state::PedalState;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/cu.usbmodem201134301"));

    let mut pedal = Pedal::open(&path)?;
    pedal.hello()?;
    match pedal.next_event(Duration::from_secs(3))? {
        PedalEvent::Connected { firmware } => println!("firmware {firmware}"),
        other => println!("unexpected: {other:?}"),
    }

    let original = fetch(&mut pedal)?;
    let original_preset = original.active_preset()?;
    println!("baseline preset {}", original_preset + 1);

    let target = if original_preset == 0 { 1 } else { 0 };
    let (frame, _) = message::set_preset(&original, target)?;
    println!(
        "sending set_preset({}) — watching all replies for 4s",
        target + 1
    );
    pedal.send_frame(&frame)?;

    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        match pedal.next_event(Duration::from_millis(250)) {
            Ok(PedalEvent::StateChanged(s)) => println!(
                "  StateChanged: preset {:?} slot {:?} ({} bytes)",
                s.active_preset(),
                s.active_slot(),
                s.len()
            ),
            Ok(PedalEvent::ParseError { raw, reason }) => {
                println!("  ParseError: {reason}");
                println!("    {} bytes: {}", raw.len(), pinex_web::hex(&raw));
            }
            Ok(other) => println!("  {other:?}"),
            Err(_) => {}
        }
    }

    // Ask explicitly — does the pedal agree it switched?
    println!("explicit request_state:");
    let after = fetch(&mut pedal)?;
    println!("  pedal reports preset {}", after.active_preset()? + 1);

    println!("restoring preset {}", original_preset + 1);
    let (restore, _) = message::set_preset(&after, original_preset)?;
    pedal.send_frame(&restore)?;
    std::thread::sleep(Duration::from_millis(600));
    let back = fetch(&mut pedal)?;
    println!("  restored to preset {}", back.active_preset()? + 1);
    Ok(())
}

fn fetch(pedal: &mut Pedal) -> Result<PedalState, Box<dyn std::error::Error>> {
    pedal.request_state()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Ok(PedalEvent::StateChanged(s)) = pedal.next_event(Duration::from_millis(250)) {
            return Ok(s);
        }
    }
    Err("no state response".into())
}
