//! Which write strategy actually sticks on this pedal?
//!
//! Staging into the inactive slot and switching to it — the approach both
//! reference implementations use — is accepted and then reverted by a pedal in
//! stomp mode (active slot C). This tries each strategy and reports which one
//! the pedal still agrees with a second later.
//!
//! Diagnostic only. Restores the original preset before exiting.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pinex_device::{Pedal, PedalEvent};
use pinex_proto::message;
use pinex_proto::state::{diff_offsets, PedalState, Slot};

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

    let target = if original_preset == 0 { 2 } else { 0 };

    for (name, build) in strategies() {
        let current = fetch(&mut pedal)?;
        let slot_before = current.active_slot()?;
        let framed = build(&current, target);

        let Some((frame, touched)) = framed else {
            println!("{name}: not applicable from slot {slot_before:?}");
            continue;
        };

        pedal.send_frame(&frame)?;
        drain(&mut pedal, Duration::from_millis(1500));

        // The question is not what the pedal said, but where it settled.
        let settled = fetch(&mut pedal)?;
        let stuck = settled.active_preset()? == target;
        println!(
            "{name}: touched {} bytes -> settled on preset {} slot {:?}  [{}]",
            touched,
            settled.active_preset()? + 1,
            settled.active_slot()?,
            if stuck { "STICKS" } else { "reverted" }
        );

        // Put it back before trying the next one.
        restore(&mut pedal, original_preset)?;
    }

    let back = fetch(&mut pedal)?;
    println!("final: preset {}", back.active_preset()? + 1);
    Ok(())
}

type Builder = fn(&PedalState, u8) -> Option<(Vec<u8>, usize)>;

fn strategies() -> Vec<(&'static str, Builder)> {
    vec![
        ("stage-into-other-slot-and-switch", |state, target| {
            message::set_preset(state, target)
                .ok()
                .map(|(f, t)| (f, t.len()))
        }),
        ("write-current-slot-in-place", |state, target| {
            let mut next = state.clone();
            let slot = next.active_slot().ok()?;
            next.set_slot_preset(slot, target).ok()?;
            next.force_direct_monitoring();
            let touched = diff_offsets(state.raw(), next.raw()).len();
            Some((message::write_state(&next), touched))
        }),
        ("write-all-three-slots", |state, target| {
            let mut next = state.clone();
            for slot in [Slot::A, Slot::B, Slot::C] {
                next.set_slot_preset(slot, target).ok()?;
            }
            next.force_direct_monitoring();
            let touched = diff_offsets(state.raw(), next.raw()).len();
            Some((message::write_state(&next), touched))
        }),
    ]
}

fn restore(pedal: &mut Pedal, preset: u8) -> Result<(), Box<dyn std::error::Error>> {
    let current = fetch(pedal)?;
    if current.active_preset()? == preset {
        return Ok(());
    }
    let mut next = current.clone();
    let slot = next.active_slot()?;
    next.set_slot_preset(slot, preset)?;
    next.force_direct_monitoring();
    pedal.send_frame(&message::write_state(&next))?;
    drain(pedal, Duration::from_millis(1200));
    Ok(())
}

fn drain(pedal: &mut Pedal, budget: Duration) {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        let _ = pedal.next_event(Duration::from_millis(100));
    }
}

fn fetch(pedal: &mut Pedal) -> Result<PedalState, Box<dyn std::error::Error>> {
    pedal.request_state()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut last = None;
    while Instant::now() < deadline {
        if let Ok(PedalEvent::StateChanged(s)) = pedal.next_event(Duration::from_millis(200)) {
            last = Some(s);
            // Keep reading briefly: the pedal can send a state then revise it.
        }
    }
    last.ok_or_else(|| "no state response".into())
}
