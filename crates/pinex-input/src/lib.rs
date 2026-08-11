//! Footswitch and button input, behind a trait so the loop runs without a Pi.
//!
//! Input emits *requests*, never state changes. A switch press asks the pedal
//! for something; the pedal remains the source of truth. That is what stops the
//! display from ever showing a preset the pedal never loaded.
//!
//! Three implementations, one seam:
//!
//! - [`ScriptedInput`] — a fixed sequence. Lets an integration test drive the
//!   whole input → command → pedal → event → display loop deterministically.
//! - [`StdinInput`] — keyboard. The same loop, on a laptop, with no hardware.
//! - `GpioInput` — interrupt-driven `rppal` on the Pi. Not built here; it is the
//!   only piece that needs the target board, and it is deliberately the *only*
//!   piece, so everything else stays testable.

pub mod debounce;

#[cfg(feature = "hat")]
pub mod hat;

use std::collections::VecDeque;
use std::io::{BufRead, IsTerminal};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

/// What the player did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// Move the browsing cursor forward, wrapping.
    Next,
    /// Move the browsing cursor back, wrapping.
    Prev,
    /// Ask the pedal to load whatever the cursor is on.
    Select,
    /// Re-sync everything from the pedal.
    Refresh,
    /// Shut down cleanly.
    Quit,
}

impl InputEvent {
    /// Parse a single-key command. Returns `None` for anything unrecognised so
    /// a stray keypress is ignored rather than acted on.
    pub fn from_key(key: &str) -> Option<Self> {
        match key.trim().to_ascii_lowercase().as_str() {
            "n" | "j" | "" => Some(Self::Next),
            "p" | "k" => Some(Self::Prev),
            "s" | "enter" => Some(Self::Select),
            "r" => Some(Self::Refresh),
            "q" => Some(Self::Quit),
            _ => None,
        }
    }
}

/// A source of player input.
pub trait InputSource: Send {
    /// Wait up to `timeout` for the next input.
    ///
    /// Returning `None` on timeout is normal and expected: the caller uses the
    /// tick to service pedal events, so input must never block the loop.
    fn poll(&mut self, timeout: Duration) -> Option<InputEvent>;
}

/// A fixed sequence of inputs, for tests.
#[derive(Debug, Default)]
pub struct ScriptedInput {
    queue: VecDeque<InputEvent>,
}

impl ScriptedInput {
    pub fn new(events: impl IntoIterator<Item = InputEvent>) -> Self {
        Self {
            queue: events.into_iter().collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl<T: InputSource + ?Sized> InputSource for Box<T> {
    fn poll(&mut self, timeout: Duration) -> Option<InputEvent> {
        (**self).poll(timeout)
    }
}

impl InputSource for ScriptedInput {
    fn poll(&mut self, timeout: Duration) -> Option<InputEvent> {
        match self.queue.pop_front() {
            Some(event) => Some(event),
            None => {
                // Honour the contract: a real source blocks for the timeout when
                // idle, and that pause is what paces the caller's loop. Returning
                // instantly would make the loop spin and starve everything else.
                std::thread::sleep(timeout);
                None
            }
        }
    }
}

/// What end-of-input means.
///
/// At a terminal, EOF is the player pressing Ctrl-D: quit. Anywhere else — a
/// pipe that closed, or `/dev/null`, which is what systemd hands a service —
/// EOF just means there is no keyboard, and the program should carry on being
/// a pedal controller.
///
/// Getting this wrong is not subtle: with `StandardInput=null` the service
/// exited in ten milliseconds and `Restart=always` turned that into a crash
/// loop. It only showed up on the Pi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndOfInput {
    /// EOF means the player asked to quit.
    Quit,
    /// EOF means there is no keyboard. Keep running, produce nothing.
    Idle,
}

/// Keyboard input on a background thread.
///
/// Reading stdin blocks, and the main loop must not, so the read lives on its
/// own thread and hands events over a channel.
pub struct StdinInput {
    rx: Receiver<InputEvent>,
    on_eof: EndOfInput,
    /// Latches once the channel closes, so we stop re-checking it.
    ended: bool,
}

impl Default for StdinInput {
    fn default() -> Self {
        Self::new()
    }
}

impl StdinInput {
    /// Read the keyboard, deciding what EOF means from whether stdin is a tty.
    pub fn new() -> Self {
        Self::with_end_of_input(if std::io::stdin().is_terminal() {
            EndOfInput::Quit
        } else {
            EndOfInput::Idle
        })
    }

    /// Build from an existing channel. Lets tests close the sender to simulate
    /// EOF, which is otherwise not reproducible — under a test harness stdin
    /// stays open, so the real reader thread never exits.
    pub fn from_receiver(rx: Receiver<InputEvent>, on_eof: EndOfInput) -> Self {
        Self {
            rx,
            on_eof,
            ended: false,
        }
    }

    pub fn with_end_of_input(on_eof: EndOfInput) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("pinex-stdin".into())
            .spawn(move || {
                let stdin = std::io::stdin();
                for line in stdin.lock().lines() {
                    let Ok(line) = line else { break };
                    if let Some(event) = InputEvent::from_key(&line) {
                        if tx.send(event).is_err() {
                            break;
                        }
                    }
                }
            })
            .expect("spawning the stdin reader");
        Self {
            rx,
            on_eof,
            ended: false,
        }
    }
}

impl InputSource for StdinInput {
    fn poll(&mut self, timeout: Duration) -> Option<InputEvent> {
        if self.ended {
            // No keyboard, and none coming. Pace the caller's loop.
            std::thread::sleep(timeout);
            return None;
        }
        match self.rx.recv_timeout(timeout) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                self.ended = true;
                match self.on_eof {
                    EndOfInput::Quit => Some(InputEvent::Quit),
                    EndOfInput::Idle => {
                        std::thread::sleep(timeout);
                        None
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_input_replays_in_order_then_reports_nothing() {
        let mut input = ScriptedInput::new([InputEvent::Next, InputEvent::Select]);
        let t = Duration::from_millis(1);

        assert_eq!(input.poll(t), Some(InputEvent::Next));
        assert_eq!(input.poll(t), Some(InputEvent::Select));
        assert_eq!(input.poll(t), None);
        assert!(input.is_empty());
    }

    /// Queued events must come back immediately; only an empty queue waits.
    /// A scripted source that slept on every event would make tests crawl.
    #[test]
    fn only_an_empty_queue_waits_out_the_timeout() {
        let mut input = ScriptedInput::new([InputEvent::Next]);
        let timeout = Duration::from_millis(80);

        let start = std::time::Instant::now();
        assert_eq!(input.poll(timeout), Some(InputEvent::Next));
        assert!(start.elapsed() < timeout, "a queued event must not wait");

        let start = std::time::Instant::now();
        assert_eq!(input.poll(timeout), None);
        assert!(start.elapsed() >= timeout, "an empty queue must wait");
    }

    /// The bug that only the Pi revealed: with stdin at /dev/null the loop must
    /// keep running, not quit. systemd gives a service exactly that, and
    /// Restart=always turns an instant exit into a crash loop.
    #[test]
    fn a_closed_stdin_does_not_quit_a_headless_run() {
        let (tx, rx) = mpsc::channel();
        let mut input = StdinInput::from_receiver(rx, EndOfInput::Idle);
        drop(tx); // stdin was /dev/null: EOF straight away
        let timeout = Duration::from_millis(20);

        for _ in 0..3 {
            assert_eq!(
                input.poll(timeout),
                None,
                "a headless run must never see Quit from EOF alone"
            );
        }
    }

    /// At a terminal, Ctrl-D still means quit.
    #[test]
    fn a_closed_stdin_quits_an_interactive_run() {
        let (tx, rx) = mpsc::channel();
        let mut input = StdinInput::from_receiver(rx, EndOfInput::Quit);
        drop(tx);
        assert_eq!(
            input.poll(Duration::from_millis(20)),
            Some(InputEvent::Quit)
        );
    }

    /// Keys already typed must still be delivered before EOF is acted on.
    #[test]
    fn buffered_keys_arrive_before_end_of_input_is_honoured() {
        let (tx, rx) = mpsc::channel();
        tx.send(InputEvent::Next).unwrap();
        tx.send(InputEvent::Select).unwrap();
        drop(tx);

        let mut input = StdinInput::from_receiver(rx, EndOfInput::Quit);
        let t = Duration::from_millis(20);
        assert_eq!(input.poll(t), Some(InputEvent::Next));
        assert_eq!(input.poll(t), Some(InputEvent::Select));
        assert_eq!(input.poll(t), Some(InputEvent::Quit));
    }

    #[test]
    fn keys_map_to_the_documented_events() {
        assert_eq!(InputEvent::from_key("n"), Some(InputEvent::Next));
        assert_eq!(InputEvent::from_key("P"), Some(InputEvent::Prev));
        assert_eq!(InputEvent::from_key(" s "), Some(InputEvent::Select));
        assert_eq!(InputEvent::from_key("r"), Some(InputEvent::Refresh));
        assert_eq!(InputEvent::from_key("q"), Some(InputEvent::Quit));
    }

    /// A bare Enter is the most common keypress; it should advance rather than
    /// do nothing.
    #[test]
    fn a_bare_enter_advances() {
        assert_eq!(InputEvent::from_key(""), Some(InputEvent::Next));
    }

    /// An unrecognised key must be ignored, not guessed at — a stray keypress
    /// should never change a preset mid-song.
    #[test]
    fn unrecognised_keys_are_ignored() {
        for key in ["x", "zzz", "1", "!"] {
            assert_eq!(InputEvent::from_key(key), None, "key {key:?}");
        }
    }
}
