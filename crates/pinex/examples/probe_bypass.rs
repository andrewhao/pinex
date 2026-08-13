//! Which byte moves when the footswitch is stomped?
//!
//! ```sh
//! ~/bin/probe_bypass /dev/ttyACM0 [seconds]
//! ```
//!
//! Put the pedal in stomp mode, run this, and stomp the footswitch on and off a
//! few times. Every state the pedal sends is diffed against the one before it,
//! byte by byte, and each difference is reported at its **end-relative** offset
//! — the addressing that survives a firmware change, and the only kind
//! `offset_from_end` is written in.
//!
//! Deliberately does not poll. The pedal is known to go silent under sustained
//! request traffic, and if it announces the footswitch at all it will do so
//! unsolicited; asking repeatedly would risk the wedge to learn nothing. One
//! request at the start, then silence.
//!
//! What this answers: whether the bypass state is in the state message at all,
//! and if so where. `BYPASS_MODE` already exists at end-relative 12, but it was
//! transcribed from `protocol.md` and never checked against hardware — it may
//! be a *setting* (true versus buffered bypass) rather than "bypassed right
//! now". A byte that tracks the footswitch is the thing worth finding.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pinex_device::{Pedal, PedalEvent};
use pinex_proto::state::{offset_from_end, PedalState};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/ttyACM0"));
    let run_for = Duration::from_secs(args.next().and_then(|s| s.parse().ok()).unwrap_or(120));

    let mut pedal = Pedal::open(&path)?;
    pedal.hello()?;

    println!("waiting for the pedal to answer...");
    let mut previous: Option<PedalState> = None;
    let deadline = Instant::now() + run_for;

    // One request, to get a baseline. Everything after this is unsolicited.
    pedal.request_state()?;

    println!(
        "\nstomp the footswitch on and off a few times; {}s\n",
        run_for.as_secs()
    );

    let mut states = 0usize;
    while Instant::now() < deadline {
        let Ok(event) = pedal.next_event(Duration::from_millis(250)) else {
            continue;
        };
        let PedalEvent::StateChanged(state) = event else {
            continue;
        };
        states += 1;
        report(&state, previous.as_ref(), states);
        previous = Some(state);
    }

    println!("\n{states} states seen in {}s", run_for.as_secs());
    if states <= 1 {
        println!(
            "The pedal sent nothing when stomped. Either it does not announce \
             the footswitch, or it was not in stomp mode. That is a real \
             finding: it would mean the UI cannot follow the pedal here without \
             polling, and polling is what wedges it."
        );
    }
    Ok(())
}

fn report(state: &PedalState, previous: Option<&PedalState>, index: usize) {
    let raw = state.raw();
    println!(
        "--- state {index}: len {}  slot {:?}  preset {:?}  stomp {:?}  \
         bypass_mode {}  cab_bypass {:?}",
        raw.len(),
        state.active_slot(),
        state.active_preset(),
        state.stomp_mode(),
        state.bypass_mode(),
        state.cab_bypass(),
    );

    let Some(previous) = previous else { return };
    let old = previous.raw();
    if old.len() != raw.len() {
        println!(
            "    length changed {} -> {}, not diffing",
            old.len(),
            raw.len()
        );
        return;
    }

    let mut changed = 0;
    for (index, (a, b)) in old.iter().zip(raw.iter()).enumerate() {
        if a == b {
            continue;
        }
        changed += 1;
        // End-relative is what the codebase addresses by, because start-relative
        // offsets shift between firmwares.
        let from_end = raw.len() - index;
        println!(
            "    byte {index:>4} (end-{from_end:<3}) {a:#04x} -> {b:#04x}{}",
            match from_end {
                offset_from_end::CURRENT_SLOT => "   <- CURRENT_SLOT",
                offset_from_end::BYPASS_MODE => "   <- BYPASS_MODE",
                offset_from_end::SLOT_A_PRESET => "   <- SLOT_A_PRESET",
                offset_from_end::SLOT_B_PRESET => "   <- SLOT_B_PRESET",
                offset_from_end::SLOT_C_PRESET => "   <- SLOT_C_PRESET",
                offset_from_end::DIRECT_MONITOR => "   <- DIRECT_MONITOR",
                _ => "",
            }
        );
    }
    if changed == 0 {
        println!("    identical to the previous state");
    }
}
