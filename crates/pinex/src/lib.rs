//! The application loop, as a library so it can be tested without hardware.
//!
//! [`App`] owns the four seams and nothing else:
//!
//! | Seam | Real | Test |
//! |---|---|---|
//! | Transport | `TtyTransport` on `/dev/cu.usbmodem*` | `PedalSim` over a PTY |
//! | Input | `StdinInput` / GPIO | `ScriptedInput` |
//! | Renderer | SPI panel / `ConsoleRenderer` | `RecordingRenderer` |
//! | Logic | `PresetBrowser` — pure either way | |
//!
//! Because all four are traits or pure data, `tests/loop.rs` drives a complete
//! browse-and-select session with no pedal attached, and `main.rs` swaps in the
//! real tty without the loop knowing.

use std::time::Duration;

use pinex_device::{Command, Pedal, PedalEvent};
use pinex_input::{InputEvent, InputSource};
use pinex_proto::message;
use pinex_proto::state::PedalState;
use pinex_ui::{PresetBrowser, Renderer};
use pinex_web::{FrameRecord, Snapshot};
use std::sync::{Arc, Mutex};

/// How long to wait for input before servicing pedal events.
const TICK: Duration = Duration::from_millis(50);

/// How often to retry opening an absent pedal. Slow enough not to hammer USB.
const RECONNECT_INTERVAL: Duration = Duration::from_secs(2);

pub struct App<I: InputSource, R: Renderer> {
    /// `None` while no pedal is open. The loop keeps running and shows
    /// "NO PEDAL" rather than exiting — a controller that dies when the pedal
    /// is unplugged would, under `Restart=always`, become a crash loop.
    pedal: Option<Pedal>,
    /// Where to look for the pedal when reconnecting.
    device: Option<std::path::PathBuf>,
    next_reconnect: std::time::Instant,
    input: I,
    renderer: R,
    browser: PresetBrowser,
    /// The pedal's own most recent state. Every write starts from these bytes;
    /// without one we cannot safely build a preset change, so we decline to try.
    last_state: Option<PedalState>,
    /// Reported rather than swallowed, and surfaced on the display.
    pub errors: Vec<String>,
    /// Shared with the debug web page, when one is running.
    snapshot: Arc<Mutex<Snapshot>>,
}

impl<I: InputSource, R: Renderer> App<I, R> {
    pub fn new(pedal: Pedal, input: I, renderer: R) -> Self {
        Self {
            pedal: Some(pedal),
            device: None,
            next_reconnect: std::time::Instant::now(),
            input,
            renderer,
            browser: PresetBrowser::new(),
            last_state: None,
            errors: Vec::new(),
            snapshot: Arc::new(Mutex::new(Snapshot::default())),
        }
    }

    /// The snapshot the debug page renders. Hand this to `DebugServer::start`.
    pub fn snapshot(&self) -> Arc<Mutex<Snapshot>> {
        Arc::clone(&self.snapshot)
    }

    /// Build with a device path, reconnecting whenever the pedal is absent.
    ///
    /// The pedal may be unplugged mid-set, or plugged in after the service
    /// starts. Neither should end the program.
    pub fn reconnecting(device: std::path::PathBuf, input: I, renderer: R) -> Self {
        Self {
            pedal: None,
            device: Some(device),
            next_reconnect: std::time::Instant::now(),
            input,
            renderer,
            browser: PresetBrowser::new(),
            last_state: None,
            errors: Vec::new(),
            snapshot: Arc::new(Mutex::new(Snapshot::default())),
        }
    }

    /// Open the conversation. The pedal answers with its firmware version,
    /// which is what triggers the initial sync.
    pub fn start(&mut self) -> std::io::Result<()> {
        match &mut self.pedal {
            Some(pedal) => pedal.hello(),
            None => Ok(()),
        }
    }

    /// Try to (re)open the pedal, at most every couple of seconds.
    fn try_reconnect(&mut self) {
        let Some(device) = self.device.clone() else {
            return;
        };
        if self.pedal.is_some() || std::time::Instant::now() < self.next_reconnect {
            return;
        }
        self.next_reconnect = std::time::Instant::now() + RECONNECT_INTERVAL;

        if !device.exists() {
            return;
        }
        if let Ok(mut pedal) = Pedal::open(&device) {
            if pedal.hello().is_ok() {
                self.pedal = Some(pedal);
                // The names we already have stay valid; the browser re-syncs
                // when the Connected event lands.
            }
        }
    }

    /// Run one iteration: input, then pedal events, then render.
    ///
    /// Returns `false` when the player asked to quit.
    pub fn step(&mut self) -> bool {
        let mut commands = Vec::new();

        if let Some(event) = self.input.poll(TICK) {
            if event == InputEvent::Quit {
                return false;
            }
            commands.extend(self.browser.handle(event));
        }

        self.try_reconnect();

        // Drain everything the pedal has said since last time.
        while let Some(event) = self
            .pedal
            .as_ref()
            .and_then(|p| p.next_event(Duration::from_millis(0)).ok())
        {
            if let PedalEvent::StateChanged(state) = &event {
                self.last_state = Some(state.clone());
            }
            if event == PedalEvent::Disconnected {
                // Drop the handle so try_reconnect will look again.
                self.pedal = None;
                self.last_state = None;
            }
            self.log_frame(&event);
            commands.extend(self.browser.apply(&event));
        }

        for command in commands {
            if let Err(e) = self.execute(command) {
                self.errors.push(e);
            }
        }

        self.renderer.render(&self.browser.view());
        self.publish_snapshot();
        true
    }

    /// Mirror the browser into the shape the debug page renders.
    fn publish_snapshot(&self) {
        let view = self.browser.view();
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        snapshot.connected = view.connection.is_connected();
        snapshot.firmware = match view.connection {
            pinex_ui::Connection::Connected { firmware } => Some(firmware.clone()),
            pinex_ui::Connection::Disconnected => None,
        };
        snapshot.active_preset = view.active;
        snapshot.active_name = view.active_name.map(str::to_string);
        snapshot.cursor = view.cursor;
        snapshot.names = (0..pinex_proto::state::MAX_PRESETS)
            .map(|i| self.browser.name_at(i).map(str::to_string))
            .collect();
        snapshot.colors = (0..pinex_proto::state::MAX_PRESETS)
            .filter_map(|i| self.browser.color_at(i))
            .collect();
    }

    /// Run until the player quits or `max_steps` elapses.
    ///
    /// The bound exists so a test can never hang the suite.
    pub fn run(&mut self, max_steps: usize) {
        for _ in 0..max_steps {
            if !self.step() {
                return;
            }
        }
    }

    /// Run for at most `budget`, so pedal round trips have time to land.
    ///
    /// Steps are paced by the input source's timeout, so a step count is a poor
    /// proxy for elapsed time; tests want the wall clock.
    pub fn settle(&mut self, budget: Duration) {
        let deadline = std::time::Instant::now() + budget;
        while std::time::Instant::now() < deadline {
            if !self.step() {
                return;
            }
        }
    }

    /// Run until `done` holds or `budget` expires. Returns whether it held.
    pub fn settle_until(
        &mut self,
        budget: Duration,
        done: impl Fn(&PresetBrowser) -> bool,
    ) -> bool {
        let deadline = std::time::Instant::now() + budget;
        while std::time::Instant::now() < deadline {
            if done(&self.browser) {
                return true;
            }
            if !self.step() {
                break;
            }
        }
        done(&self.browser)
    }

    /// Record every event on the debug page, errors with their raw bytes.
    ///
    /// This is the point of the page: a firmware change shows up here as hex,
    /// not as a pedal that mysteriously stops responding.
    fn log_frame(&self, event: &PedalEvent) {
        let record = match event {
            PedalEvent::Connected { firmware } => FrameRecord {
                summary: format!("Connected — firmware {firmware}"),
                raw_hex: String::new(),
                is_error: false,
            },
            PedalEvent::Disconnected => FrameRecord {
                summary: "Disconnected".into(),
                raw_hex: String::new(),
                is_error: true,
            },
            PedalEvent::StateChanged(state) => FrameRecord {
                summary: format!(
                    "StateChanged — preset {:?}, slot {:?}",
                    state.active_preset(),
                    state.active_slot()
                ),
                raw_hex: pinex_web::hex(state.raw()),
                is_error: false,
            },
            PedalEvent::PresetName(info) => FrameRecord {
                summary: format!("Preset {:02} — {}", info.index + 1, info.name),
                raw_hex: String::new(),
                is_error: false,
            },
            PedalEvent::WriteAcknowledged => FrameRecord {
                summary: "write acknowledged (not yet confirmed)".into(),
                raw_hex: String::new(),
                is_error: false,
            },
            PedalEvent::ParseError { raw, reason } => FrameRecord {
                summary: reason.clone(),
                raw_hex: pinex_web::hex(raw),
                is_error: true,
            },
        };
        self.snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_frame(record);
    }

    fn execute(&mut self, command: Command) -> Result<(), String> {
        let Some(pedal) = self.pedal.as_mut() else {
            return Err("no pedal connected".to_string());
        };
        match command {
            Command::RequestState => pedal.request_state().map_err(|e| e.to_string()),
            Command::RequestPreset(n) => pedal.request_preset(n).map_err(|e| e.to_string()),
            Command::SetPreset(n) => {
                // Refuse rather than guess: a write built from a state we do not
                // have would be a write of bytes we invented.
                let Some(current) = &self.last_state else {
                    return Err(format!(
                        "cannot set preset {n}: no state received from the pedal yet"
                    ));
                };
                let (frame, _touched) =
                    message::set_preset(current, n).map_err(|e| e.to_string())?;
                pedal.send_frame(&frame).map_err(|e| e.to_string())
            }
        }
    }

    /// Feed one input and run a step, for tests and scripted sessions.
    pub fn step_with(&mut self, input: InputEvent) -> bool {
        let commands = self.browser.handle(input);
        for command in commands {
            if let Err(e) = self.execute(command) {
                self.errors.push(e);
            }
        }
        self.step()
    }

    /// Mark the browser connected without a handshake.
    ///
    /// Exists so a test can reach the "connected but no state yet" window, which
    /// is otherwise a race. Not used by the binary.
    pub fn force_connected_for_test(&mut self) {
        self.browser.apply(&PedalEvent::Connected {
            firmware: "test".into(),
        });
    }

    pub fn browser(&self) -> &PresetBrowser {
        &self.browser
    }

    pub fn renderer(&self) -> &R {
        &self.renderer
    }
}
