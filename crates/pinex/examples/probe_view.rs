//! Print exactly the inputs the panel uses to decide what to draw.
//!
//! Diagnostic: the physical pedal shows one colour while the panel draws that
//! box dimmed, so the question is whether we are reading the wrong slot, the
//! wrong colour, or applying the wrong lit flag.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pinex_device::{Pedal, PedalEvent};
use pinex_proto::state::{PedalState, Slot};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/ttyACM0"));

    let mut pedal = Pedal::open(&path)?;
    pedal.hello()?;
    let _ = pedal.next_event(Duration::from_secs(3));
    pedal.request_state()?;

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut state: Option<PedalState> = None;
    while Instant::now() < deadline {
        if let Ok(PedalEvent::StateChanged(s)) = pedal.next_event(Duration::from_millis(200)) {
            state = Some(s);
        }
    }
    let state = state.ok_or("no state")?;

    println!("stomp_mode = {:?}", state.stomp_mode());
    println!("active_slot = {:?}", state.active_slot());
    println!("active_preset = {:?}", state.active_preset());
    for slot in [Slot::A, Slot::B, Slot::C] {
        println!("  slot {slot:?} holds preset {}", state.slot_preset(slot));
    }

    let colors = state.preset_colors()?;
    println!("\ncolours the pedal reports:");
    for (index, rgb) in colors.iter().enumerate() {
        let marker = if Some(index as u8) == state.active_preset().ok() {
            " <- ACTIVE"
        } else {
            ""
        };
        println!(
            "  {:02} rgb({:3},{:3},{:3}){marker}",
            index + 1,
            rgb[0],
            rgb[1],
            rgb[2]
        );
    }
    Ok(())
}
