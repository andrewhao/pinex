//! The whole application, driven end to end with no hardware attached.
//!
//! Input is scripted, the pedal is simulated over a real PTY replaying captured
//! bytes, and the display is recorded. Every seam is the same one production
//! uses; only the ends are swapped. If this passes, the only untested thing left
//! is the physical device.

use std::time::Duration;

use pinex::App;
use pinex_device::sim::PedalSim;
use pinex_device::Pedal;
use pinex_input::{InputEvent, ScriptedInput};
use pinex_proto::state::Slot;
use pinex_ui::RecordingRenderer;

/// Wall-clock budget for PTY round trips to land. Generous, but the tests exit
/// as soon as the condition holds, so the usual cost is far lower.
const BUDGET: Duration = Duration::from_secs(5);

fn app_with(
    inputs: impl IntoIterator<Item = InputEvent>,
) -> (PedalSim, App<ScriptedInput, RecordingRenderer>) {
    let sim = PedalSim::start().unwrap();
    let pedal = Pedal::open(sim.device_path()).unwrap();
    let app = App::new(
        pedal,
        ScriptedInput::new(inputs),
        RecordingRenderer::default(),
    );
    (sim, app)
}

/// Connecting must populate the display with the truth: firmware, the preset
/// actually playing, and its name.
#[test]
fn starting_up_shows_the_firmware_and_what_the_pedal_is_playing() {
    let (_sim, mut app) = app_with([]);
    app.start().unwrap();
    app.settle_until(BUDGET, |b| b.view().active_name.is_some());

    let view = app.browser().view();
    assert!(view.connection.is_connected());
    assert_eq!(
        view.active,
        Some(1),
        "the pedal was on preset 2 (0-based 1)"
    );
    assert_eq!(
        view.active_name,
        Some("TF MORNIING GLORY - BRIGHT 1"),
        "names come from the pedal, typos and all"
    );

    let screen = app.renderer().last().expect("something must be rendered");
    assert_eq!(screen[0], "Tonex ONE  fw 1.3.17");
    assert_eq!(screen[1], "NOW  02 TF MORNIING GLORY - BRIGHT 1");
    assert!(app.errors.is_empty(), "errors: {:?}", app.errors);
}

/// The whole point of a browser: walk the list and see real names.
#[test]
fn browsing_walks_the_presets_and_shows_their_real_names() {
    let (_sim, mut app) = app_with([]);
    app.start().unwrap();
    app.settle_until(BUDGET, |b| (0..20).all(|i| b.name_at(i).is_some()));

    // Park the cursor at 0, then walk forward reading names off the display.
    let mut names = Vec::new();
    for _ in 0..20 {
        let view = app.browser().view();
        names.push((view.cursor, view.cursor_name.map(str::to_string)));
        app.step_with(InputEvent::Down);
    }

    assert!(
        names.iter().all(|(_, name)| name.is_some()),
        "every preset should have a name by now: {names:?}"
    );
    let all: Vec<String> = names.iter().map(|(_, n)| n.clone().unwrap()).collect();
    assert!(all.contains(&"TF BENSON PREAMP - 1".to_string()));
    assert!(all.contains(&"TF TILT - 1 ADV".to_string()));
    assert_eq!(all.len(), 20);
}

/// Select must reach the pedal, and the display must only believe it once the
/// pedal confirms.
#[test]
fn selecting_a_preset_changes_what_the_pedal_plays() {
    let (sim, mut app) = app_with([]);
    app.start().unwrap();
    app.settle_until(BUDGET, |b| b.view().active.is_some());
    assert_eq!(sim.active_preset(), 1);

    // Browse to preset 8 (0-based 7) and select it.
    for _ in 0..6 {
        app.step_with(InputEvent::Down);
    }
    assert_eq!(app.browser().view().cursor, 7);

    app.step_with(InputEvent::Select);
    app.settle_until(BUDGET, |b| b.view().active == Some(7));

    assert_eq!(sim.active_preset(), 7, "the pedal must have switched");
    assert_eq!(sim.writes_accepted(), 1, "exactly one write");
    assert_eq!(app.browser().view().active, Some(7));
    assert!(
        !app.browser().view().pending,
        "the pedal confirmed, so nothing is in flight"
    );
    assert!(app.errors.is_empty(), "errors: {:?}", app.errors);
}

/// Walking forward from the last preset must wrap, not stall or panic.
#[test]
fn the_browser_wraps_around_the_end_of_the_list() {
    let (_sim, mut app) = app_with([]);
    app.start().unwrap();
    app.settle_until(BUDGET, |b| b.view().active.is_some());

    for _ in 0..20 {
        app.step_with(InputEvent::Down);
    }
    let full_circle = app.browser().view().cursor;

    app.step_with(InputEvent::Up);
    assert_eq!(
        app.browser().view().cursor,
        (full_circle + 19) % 20,
        "prev must step back from wherever a full lap landed"
    );
}

/// A Select before any state has arrived must be refused with an explanation,
/// never guessed at — a write built from a state we do not have is a write of
/// bytes we invented.
#[test]
fn selecting_before_the_state_arrives_is_refused_not_guessed() {
    let sim = PedalSim::start().unwrap();
    let pedal = Pedal::open(sim.device_path()).unwrap();
    let mut app = App::new(pedal, ScriptedInput::new([]), RecordingRenderer::default());

    // Force "connected" without letting a state response land.
    app.force_connected_for_test();
    app.step_with(InputEvent::Select);

    assert_eq!(sim.writes_accepted(), 0, "nothing may be transmitted");
    assert!(
        app.errors.iter().any(|e| e.contains("no state")),
        "the refusal must say why: {:?}",
        app.errors
    );
}

/// Quit must end the loop rather than spin.
#[test]
fn quitting_stops_the_loop() {
    let (_sim, mut app) = app_with([InputEvent::Quit]);
    app.start().unwrap();
    let started = std::time::Instant::now();
    app.settle(BUDGET);
    assert!(
        started.elapsed() < BUDGET,
        "Quit should end the loop early, but it ran the full budget"
    );
}

/// The loop ticks 20 times a second. Rendering every tick filled the Pi's
/// journal with identical lines, and would be wasted SPI traffic on a panel.
#[test]
fn an_idle_view_redraws_only_at_the_animation_rate() {
    let sim = PedalSim::start().unwrap();
    let pedal = Pedal::open(sim.device_path()).unwrap();
    let mut app = App::new(pedal, ScriptedInput::new([]), RecordingRenderer::default());
    app.start().unwrap();
    app.settle_until(BUDGET, |b| b.view().active_name.is_some());

    // Let it idle. Names on this rig are long enough to scroll, so some
    // redrawing is expected and correct — but it must be paced by the animation
    // clock (~5 a second), not by the 20 Hz loop tick. The bug this guards
    // against filled the Pi's journal with identical lines many times a second.
    let before = app.renderer().frames.len();
    app.settle(Duration::from_millis(600));
    let drawn = app.renderer().frames.len() - before;

    assert!(
        drawn <= 6,
        "an idle loop drew {drawn} frames in 600ms; the animation clock should cap it near 3"
    );

    // ...but a real change must still draw, immediately.
    let before_input = app.renderer().frames.len();
    app.step_with(InputEvent::Down);
    assert!(
        app.renderer().frames.len() > before_input,
        "moving the cursor must redraw"
    );
}

/// A missing pedal is not an error: the loop must keep running and say
/// NO PEDAL. Under systemd, exiting here would become a crash loop.
#[test]
fn a_missing_pedal_keeps_the_loop_alive_showing_no_pedal() {
    let mut app = App::reconnecting(
        std::path::PathBuf::from("/nonexistent/tonex"),
        ScriptedInput::new([]),
        RecordingRenderer::default(),
    );
    app.start().unwrap();

    let started = std::time::Instant::now();
    app.settle(Duration::from_millis(600));

    assert!(
        started.elapsed() >= Duration::from_millis(500),
        "the loop must keep running rather than exit"
    );
    let screen = app.renderer().last().expect("must still render");
    assert_eq!(screen[0], "NO PEDAL");
    assert!(!app.browser().view().connection.is_connected());
}

/// A pedal that appears after start-up must be picked up without a restart.
#[test]
fn a_pedal_plugged_in_later_is_picked_up() {
    let sim = PedalSim::start().unwrap();
    let mut app = App::reconnecting(
        sim.device_path().to_path_buf(),
        ScriptedInput::new([]),
        RecordingRenderer::default(),
    );
    app.start().unwrap();

    assert!(
        app.settle_until(BUDGET, |b| b.view().connection.is_connected()),
        "should have connected on its own"
    );
    assert!(app.settle_until(BUDGET, |b| b.view().active.is_some()));
    assert_eq!(app.browser().view().active, Some(1));
}

/// The headline requirement: choose what is in A and what is in B, and step
/// between them, without either assignment disturbing the other.
#[test]
fn slots_can_be_assigned_independently_and_stepped_between() {
    let (sim, mut app) = app_with([]);
    app.start().unwrap();
    app.settle_until(BUDGET, |b| b.view().active.is_some());

    // The captured pedal is in stomp mode. Paging cycles Slots -> Stomp -> Gain,
    // so walk round to the A/B page; arriving there must take the *pedal* with
    // it, not just the display.
    for _ in 0..3 {
        if app.browser().view().screen == pinex_ui::browser::Screen::Slots {
            break;
        }
        app.step_with(InputEvent::Page);
        app.settle(Duration::from_millis(300));
    }
    assert_eq!(
        app.browser().view().screen,
        pinex_ui::browser::Screen::Slots,
        "should have reached the A/B page"
    );
    assert!(
        app.settle_until(BUDGET, |b| !b.view().stomp_mode),
        "arriving on A/B must put the pedal in A/B mode"
    );

    // Put preset 3 into whichever slot is selected, and go there.
    let first = app.browser().view().selected;
    for _ in 0..3 {
        app.step_with(InputEvent::Down);
    }
    let want_first = app.browser().view().cursor;
    app.step_with(InputEvent::Select);
    assert!(
        app.settle_until(BUDGET, |b| b.view().slot_preset(first) == Some(want_first)),
        "slot {first:?} should hold preset {want_first}"
    );

    // Switch to editing the other slot and give it something different.
    // Directional: A is drawn on the left, B on the right.
    let other_side = if first == Slot::A {
        InputEvent::Right
    } else {
        InputEvent::Left
    };
    app.step_with(other_side);
    let second = app.browser().view().selected;
    assert_ne!(second, first, "the other side must select the other slot");

    for _ in 0..5 {
        app.step_with(InputEvent::Down);
    }
    let want_second = app.browser().view().cursor;
    app.step_with(InputEvent::Select);
    assert!(
        app.settle_until(BUDGET, |b| b.view().slot_preset(second)
            == Some(want_second)),
        "slot {second:?} should hold preset {want_second}"
    );

    // The first slot must still hold what it was given — assigning one slot
    // may never disturb the other.
    let view = app.browser().view();
    assert_eq!(
        view.slot_preset(first),
        Some(want_first),
        "assigning {second:?} disturbed {first:?}"
    );
    assert_eq!(sim.active_preset(), want_second, "the pedal followed");
    assert!(app.errors.is_empty(), "errors: {:?}", app.errors);
}

/// Gain is the one control that applies as it moves, like a knob.
#[test]
fn input_trim_changes_reach_the_pedal() {
    let (_sim, mut app) = app_with([]);
    app.start().unwrap();
    app.settle_until(BUDGET, |b| b.view().active.is_some());

    // Page around to Gain.
    for _ in 0..3 {
        app.step_with(InputEvent::Page);
        if app.browser().view().screen == pinex_ui::browser::Screen::Levels {
            break;
        }
    }
    assert_eq!(
        app.browser().view().screen,
        pinex_ui::browser::Screen::Levels
    );

    // The page opens on the output level, which is what a player rides; trim
    // is the row below it.
    app.step_with(InputEvent::Down);
    assert_eq!(
        app.browser().view().level_focus,
        pinex_ui::browser::Level::Trim
    );

    let before = app.browser().view().gain_db;
    for _ in 0..4 {
        app.step_with(InputEvent::Right);
    }
    app.settle(Duration::from_millis(400));

    let after = app.browser().view().gain_db;
    assert!(
        after > before,
        "gain should have risen: {before} -> {after}"
    );
    assert!(app.errors.is_empty(), "errors: {:?}", app.errors);
}

/// The display must never claim a preset is playing when the pedal is gone.
#[test]
fn losing_the_pedal_shows_no_pedal_and_drops_the_active_preset() {
    let (sim, mut app) = app_with([]);
    app.start().unwrap();
    app.settle_until(BUDGET, |b| b.view().active.is_some());
    assert!(app.browser().view().active.is_some());

    drop(sim); // unplug
    app.settle_until(BUDGET, |b| !b.view().connection.is_connected());

    let view = app.browser().view();
    assert!(
        !view.connection.is_connected(),
        "must notice the pedal left"
    );
    assert_eq!(view.active, None, "must not claim to know what is playing");

    let screen = app.renderer().last().unwrap();
    assert_eq!(screen[0], "NO PEDAL");
    assert_eq!(screen[1], "NOW  --");
}
