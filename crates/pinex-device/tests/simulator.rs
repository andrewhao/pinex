//! End-to-end: a `Pedal` talking to a simulated pedal over a real PTY.
//!
//! Everything below runs through the real tty open, the real termios setup, the
//! real reader thread, the real frame accumulator and the real codec. The bytes
//! the simulator replies with were captured from an actual Tonex ONE.
//!
//! What this proves: our plumbing carries real pedal bytes correctly, end to
//! end. What it does not prove: that a real pedal *accepts* what we send. The
//! simulator recognises our requests because they are ours.

use std::time::Duration;

use pinex_device::sim::PedalSim;
use pinex_device::{Pedal, PedalEvent};
use pinex_proto::state::Slot;

const TIMEOUT: Duration = Duration::from_secs(3);

/// Values read off the real pedal at capture time.
const CAPTURED_FIRMWARE: &str = "1.3.17";
const CAPTURED_ACTIVE_PRESET: u8 = 1;

fn sim_pedal() -> (PedalSim, Pedal) {
    let sim = PedalSim::start().unwrap();
    let pedal = Pedal::open(sim.device_path()).unwrap();
    (sim, pedal)
}

#[test]
fn the_handshake_reports_the_captured_firmware_version() {
    let (_sim, mut pedal) = sim_pedal();
    pedal.hello().unwrap();

    match pedal.next_event(TIMEOUT).unwrap() {
        PedalEvent::Connected { firmware } => assert_eq!(firmware, CAPTURED_FIRMWARE),
        other => panic!("expected Connected, got {other:?}"),
    }
}

#[test]
fn requesting_state_yields_the_pedals_real_state() {
    let (_sim, mut pedal) = sim_pedal();
    pedal.request_state().unwrap();

    match pedal.next_event(TIMEOUT).unwrap() {
        PedalEvent::StateChanged(state) => {
            assert_eq!(state.active_slot().unwrap(), Slot::C);
            assert_eq!(state.active_preset().unwrap(), CAPTURED_ACTIVE_PRESET);
            assert_eq!(state.slot_preset(Slot::A), 15);
            assert_eq!(state.slot_preset(Slot::B), 16);
            assert_eq!(state.tuning_reference_hz(), 440);
        }
        other => panic!("expected StateChanged, got {other:?}"),
    }
}

/// Every preset name, fetched the way the real app fetches them.
#[test]
fn all_twenty_preset_names_can_be_read_over_the_wire() {
    let (_sim, mut pedal) = sim_pedal();

    let mut names = vec![None; 20];
    for preset in 0..20u8 {
        pedal.request_preset(preset).unwrap();
        match pedal.next_event(TIMEOUT).unwrap() {
            PedalEvent::PresetName(info) => {
                assert_eq!(info.index, preset);
                names[preset as usize] = Some(info.name);
            }
            other => panic!("preset {preset}: expected PresetName, got {other:?}"),
        }
    }

    assert!(
        names.iter().all(Option::is_some),
        "every preset must answer"
    );
    assert_eq!(names[0].as_deref(), Some("TF BENSON PREAMP - 1"));
    assert_eq!(names[15].as_deref(), Some("TF TILT - 1 ADV"));
}

/// The point of the simulator: a write must be observable, not just accepted.
#[test]
fn staging_a_preset_changes_what_the_simulator_is_playing() {
    let (sim, mut pedal) = sim_pedal();

    pedal.request_state().unwrap();
    let state = match pedal.next_event(TIMEOUT).unwrap() {
        PedalEvent::StateChanged(state) => state,
        other => panic!("expected StateChanged, got {other:?}"),
    };
    assert_eq!(sim.active_preset(), CAPTURED_ACTIVE_PRESET);

    // Built and verified by pinex-proto, exactly as the app builds it.
    let (frame, _) = pinex_proto::message::set_preset(&state, 7).unwrap();
    pedal.send_frame(&frame).unwrap();

    match pedal.next_event(TIMEOUT).unwrap() {
        PedalEvent::StateChanged(echoed) => assert_eq!(echoed.active_preset().unwrap(), 7),
        other => panic!("expected StateChanged, got {other:?}"),
    }

    assert_eq!(sim.active_preset(), 7, "the write must be observable");
    assert_eq!(sim.writes_accepted(), 1);
    // The pedal was on slot C, which stages back into A.
    assert_eq!(sim.active_slot().unwrap(), Slot::A);
}

/// Repeated changes must alternate slots so the slot being heard is never
/// overwritten while it plays — the glitch-free path, over the wire.
#[test]
fn successive_preset_changes_alternate_slots_over_the_wire() {
    let (sim, mut pedal) = sim_pedal();

    pedal.request_state().unwrap();
    let mut state = match pedal.next_event(TIMEOUT).unwrap() {
        PedalEvent::StateChanged(state) => state,
        other => panic!("expected StateChanged, got {other:?}"),
    };

    let mut slots = Vec::new();
    for preset in [3u8, 11, 19, 0] {
        let (frame, _) = pinex_proto::message::set_preset(&state, preset).unwrap();
        pedal.send_frame(&frame).unwrap();

        state = match pedal.next_event(TIMEOUT).unwrap() {
            PedalEvent::StateChanged(echoed) => echoed,
            other => panic!("expected StateChanged, got {other:?}"),
        };

        assert_eq!(sim.active_preset(), preset);
        slots.push(sim.active_slot().unwrap());
    }

    // C stages into A, then A↔B from there.
    assert_eq!(slots, vec![Slot::A, Slot::B, Slot::A, Slot::B]);
}

/// The simulator must not invent replies for requests it has no capture for.
/// We only ever captured `Summary` preset requests, never `Full`.
#[test]
fn the_simulator_refuses_to_answer_a_request_it_has_no_capture_for() {
    let (sim, mut pedal) = sim_pedal();

    let full =
        pinex_proto::message::request_preset(0, pinex_proto::message::PresetDetail::Full).unwrap();
    pedal.send_frame(&full).unwrap();

    assert!(
        pedal.next_event(Duration::from_millis(500)).is_err(),
        "no reply should be fabricated for a request we never captured"
    );
    assert_eq!(sim.unanswered_requests(), 1);
}
