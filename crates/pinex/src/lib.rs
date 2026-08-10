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

/// How long to wait for input before servicing pedal events.
const TICK: Duration = Duration::from_millis(50);

pub struct App<I: InputSource, R: Renderer> {
    pedal: Pedal,
    input: I,
    renderer: R,
    browser: PresetBrowser,
    /// The pedal's own most recent state. Every write starts from these bytes;
    /// without one we cannot safely build a preset change, so we decline to try.
    last_state: Option<PedalState>,
    /// Reported rather than swallowed, and surfaced on the display.
    pub errors: Vec<String>,
}

impl<I: InputSource, R: Renderer> App<I, R> {
    pub fn new(pedal: Pedal, input: I, renderer: R) -> Self {
        Self {
            pedal,
            input,
            renderer,
            browser: PresetBrowser::new(),
            last_state: None,
            errors: Vec::new(),
        }
    }

    /// Open the conversation. The pedal answers with its firmware version,
    /// which is what triggers the initial sync.
    pub fn start(&mut self) -> std::io::Result<()> {
        self.pedal.hello()
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

        // Drain everything the pedal has said since last time.
        while let Ok(event) = self.pedal.next_event(Duration::from_millis(0)) {
            if let PedalEvent::StateChanged(state) = &event {
                self.last_state = Some(state.clone());
            }
            commands.extend(self.browser.apply(&event));
        }

        for command in commands {
            if let Err(e) = self.execute(command) {
                self.errors.push(e);
            }
        }

        self.renderer.render(&self.browser.view());
        true
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

    fn execute(&mut self, command: Command) -> Result<(), String> {
        match command {
            Command::RequestState => self.pedal.request_state().map_err(|e| e.to_string()),
            Command::RequestPreset(n) => self.pedal.request_preset(n).map_err(|e| e.to_string()),
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
                self.pedal.send_frame(&frame).map_err(|e| e.to_string())
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
