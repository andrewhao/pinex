//! End-to-end: a `Pedal` talking to a simulated pedal over a real PTY.
//!
//! **What this proves and what it does not.** Everything below runs through the
//! real tty open, the real termios setup, the real reader thread, the real
//! frame accumulator, and the real codec. The bytes the simulator *replies*
//! with are captured hardware bytes for Hello, and captured hardware body bytes
//! inside our own framing for state.
//!
//! So a green run proves the plumbing carries real bytes correctly. It does not
//! prove a real pedal accepts our requests or that we parse real state
//! responses — the state framing here is ours, not IK Multimedia's. Only a
//! capture from an actual pedal can close that gap.

use std::time::Duration;

use pinex_device::sim::PedalSim;
use pinex_device::{Pedal, PedalEvent};
use pinex_proto::state::Slot;

const TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn the_handshake_reports_the_captured_firmware_version() {
    let sim = PedalSim::start().unwrap();
    let mut pedal = Pedal::open(sim.device_path()).unwrap();

    pedal.hello().unwrap();

    match pedal.next_event(TIMEOUT).unwrap() {
        PedalEvent::Connected { firmware } => assert_eq!(firmware, "1.1.3"),
        other => panic!("expected Connected, got {other:?}"),
    }
}

#[test]
fn requesting_state_yields_the_captured_state() {
    let sim = PedalSim::start().unwrap();
    let mut pedal = Pedal::open(sim.device_path()).unwrap();

    pedal.request_state().unwrap();

    match pedal.next_event(TIMEOUT).unwrap() {
        PedalEvent::StateChanged(state) => {
            // Values annotated in the captured state dump.
            assert_eq!(state.active_slot().unwrap(), Slot::A);
            assert_eq!(state.active_preset().unwrap(), 0);
            assert_eq!(state.slot_preset(Slot::B), 2);
        }
        other => panic!("expected StateChanged, got {other:?}"),
    }
}

/// The point of the simulator: a write must be observable, not just accepted.
#[test]
fn staging_a_preset_changes_what_the_simulator_is_playing() {
    let sim = PedalSim::start().unwrap();
    let mut pedal = Pedal::open(sim.device_path()).unwrap();

    pedal.request_state().unwrap();
    let mut state = match pedal.next_event(TIMEOUT).unwrap() {
        PedalEvent::StateChanged(state) => state,
        other => panic!("expected StateChanged, got {other:?}"),
    };
    assert_eq!(sim.active_preset(), 0, "simulator starts on preset 0");

    state.stage_preset_in_inactive_slot(7).unwrap();
    pedal.write_state(&state).unwrap();

    // The pedal echoes the state it adopted, exactly as the real one does.
    match pedal.next_event(TIMEOUT).unwrap() {
        PedalEvent::StateChanged(echoed) => {
            assert_eq!(echoed.active_slot().unwrap(), Slot::B);
            assert_eq!(echoed.active_preset().unwrap(), 7);
        }
        other => panic!("expected StateChanged, got {other:?}"),
    }

    assert_eq!(sim.active_preset(), 7, "the write must be observable");
    assert_eq!(sim.active_slot().unwrap(), Slot::B);
}

/// Repeated changes must alternate slots, so the slot being heard is never
/// overwritten while it plays. This is the glitch-free path from the design doc,
/// exercised over the wire rather than in a unit test.
#[test]
fn successive_preset_changes_alternate_slots_over_the_wire() {
    let sim = PedalSim::start().unwrap();
    let mut pedal = Pedal::open(sim.device_path()).unwrap();

    pedal.request_state().unwrap();
    let mut state = match pedal.next_event(TIMEOUT).unwrap() {
        PedalEvent::StateChanged(state) => state,
        other => panic!("expected StateChanged, got {other:?}"),
    };

    let mut slots = Vec::new();
    for preset in [3u8, 11, 19, 0] {
        state.stage_preset_in_inactive_slot(preset).unwrap();
        pedal.write_state(&state).unwrap();

        state = match pedal.next_event(TIMEOUT).unwrap() {
            PedalEvent::StateChanged(echoed) => echoed,
            other => panic!("expected StateChanged, got {other:?}"),
        };

        assert_eq!(sim.active_preset(), preset);
        slots.push(sim.active_slot().unwrap());
    }

    assert_eq!(slots, vec![Slot::B, Slot::A, Slot::B, Slot::A]);
}

/// The simulator must not invent replies for requests it has no captured
/// evidence for — a silent, plausible-looking fake is worse than nothing.
#[test]
fn the_simulator_refuses_to_fabricate_a_preset_response() {
    let sim = PedalSim::start().unwrap();
    let mut pedal = Pedal::open(sim.device_path()).unwrap();

    pedal.request_preset(0).unwrap();

    assert!(
        pedal.next_event(Duration::from_millis(500)).is_err(),
        "no reply should be fabricated for an unconfirmed message type"
    );
    assert_eq!(sim.unanswered_requests(), 1);
}
