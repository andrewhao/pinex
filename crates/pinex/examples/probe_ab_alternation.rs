//! Does the A/B double-buffering actually alternate on hardware?
//!
//! The reason the A/B route exists is that loading a preset into the slot being
//! heard can be audible. That only holds if successive changes really do land in
//! the slot that is *not* playing. This walks several presets and reports where
//! each one settled.
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
    let _ = pedal.next_event(Duration::from_secs(3));

    let original = fetch(&mut pedal)?;
    let original_preset = original.active_preset()?;
    println!(
        "baseline: preset {} in slot {:?}",
        original_preset + 1,
        original.active_slot()?
    );

    let mut slots = Vec::new();
    let mut ok = true;

    for target in [2u8, 5, 9, 3] {
        let before = fetch(&mut pedal)?;
        let slot_before = before.active_slot()?;
        let preset_in_playing_slot = before.slot_preset(slot_before);

        let (frame, _) = message::set_preset(&before, target)?;
        pedal.send_frame(&frame)?;
        std::thread::sleep(Duration::from_millis(1500));

        let after = fetch(&mut pedal)?;
        let slot_after = after.active_slot()?;
        let landed = after.active_preset()? == target;
        let moved_slot = slot_after != slot_before;
        // The slot we were hearing must still hold what it held.
        let untouched = after.slot_preset(slot_before) == preset_in_playing_slot;

        println!(
            "preset {} -> {:?} (was {:?})  landed={landed} moved_slot={moved_slot} \
             previous_slot_intact={untouched}",
            target + 1,
            slot_after,
            slot_before
        );
        ok &= landed && moved_slot && untouched;
        slots.push(slot_after);
    }

    println!("slot sequence: {slots:?}");
    println!(
        "VERDICT: {}",
        if ok {
            "A/B double-buffering confirmed on hardware"
        } else {
            "A/B double-buffering DOES NOT hold — see rows above"
        }
    );

    // Put it back.
    let current = fetch(&mut pedal)?;
    let (restore, _) = message::set_preset(&current, original_preset)?;
    pedal.send_frame(&restore)?;
    std::thread::sleep(Duration::from_millis(1500));
    let back = fetch(&mut pedal)?;
    println!("restored to preset {}", back.active_preset()? + 1);
    Ok(())
}

fn fetch(pedal: &mut Pedal) -> Result<PedalState, Box<dyn std::error::Error>> {
    pedal.request_state()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut last = None;
    while Instant::now() < deadline {
        if let Ok(PedalEvent::StateChanged(s)) = pedal.next_event(Duration::from_millis(200)) {
            last = Some(s);
        }
    }
    last.ok_or_else(|| "no state response".into())
}
