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
        app.step_with(InputEvent::Next);
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
        app.step_with(InputEvent::Next);
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
        app.step_with(InputEvent::Next);
    }
    let full_circle = app.browser().view().cursor;

    app.step_with(InputEvent::Prev);
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
fn an_unchanged_view_is_not_re_rendered() {
    let sim = PedalSim::start().unwrap();
    let pedal = Pedal::open(sim.device_path()).unwrap();
    let mut app = App::new(pedal, ScriptedInput::new([]), RecordingRenderer::default());
    app.start().unwrap();
    app.settle_until(BUDGET, |b| b.view().active_name.is_some());

    // Let it idle: nothing changes, so nothing new should be drawn.
    let before = app.renderer().frames.len();
    app.settle(Duration::from_millis(600));
    let after = app.renderer().frames.len();

    assert_eq!(
        before,
        after,
        "an idle loop drew {} extra identical frames",
        after - before
    );

    // ...but a real change must still draw.
    app.step_with(InputEvent::Next);
    assert!(
        app.renderer().frames.len() > after,
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
