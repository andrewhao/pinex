//! The Waveshare 1.44" HAT's joystick and three keys, as an [`InputSource`].
//!
//! Lives here rather than beside the display driver because it is input: the
//! app layer should not have to reach into the UI crate to read a button.
//!
//! All inputs are active-low with the internal pull-up engaged, which is how
//! the HAT wires them: pressed pulls the pin to ground.

use std::collections::VecDeque;
use std::time::Duration;

use rppal::gpio::{Gpio, InputPin};

use crate::debounce::Debouncer;
use crate::{InputEvent, InputSource, BINDINGS};

/// BCM pin numbers, from the vendor's demo code.
pub mod pins {
    pub const JOY_UP: u8 = 6;
    pub const JOY_DOWN: u8 = 19;
    pub const JOY_LEFT: u8 = 5;
    pub const JOY_RIGHT: u8 = 26;
    pub const JOY_PRESS: u8 = 13;
    pub const KEY1: u8 = 21;
    pub const KEY2: u8 = 20;
    pub const KEY3: u8 = 16;
}

pub struct HatButtons {
    pins: Vec<InputPin>,
    /// Requires a level to hold steady before believing it. See
    /// [`crate::debounce`] for why the naive version fired on noise.
    debounce: Debouncer<8>,
    /// Presses seen but not yet returned.
    ///
    /// The debouncer settles the whole eight-bit vector at once, so two buttons
    /// pressed within the same window arrive together. Returning one and
    /// discarding the rest lost them for good — they could not re-fire until
    /// physically released. They queue instead.
    pending: VecDeque<usize>,
}

impl HatButtons {
    pub fn open() -> Result<Self, rppal::gpio::Error> {
        let gpio = Gpio::new()?;
        let order = [
            pins::JOY_UP,
            pins::JOY_DOWN,
            pins::JOY_LEFT,
            pins::JOY_RIGHT,
            pins::JOY_PRESS,
            pins::KEY1,
            pins::KEY2,
            pins::KEY3,
        ];
        let mut opened = Vec::with_capacity(order.len());
        for pin in order {
            opened.push(gpio.get(pin)?.into_input_pullup());
        }
        Ok(Self {
            pins: opened,
            debounce: Debouncer::new(),
            pending: VecDeque::new(),
        })
    }

    /// Raw pin levels, undebounced. `true` means pressed (the pin is low).
    ///
    /// Diagnostic: it is the only way to tell "the debounce ate it" apart from
    /// "rppal never saw it".
    pub fn raw_levels(&self) -> [bool; 8] {
        let mut levels = [false; 8];
        for (slot, pin) in levels.iter_mut().zip(self.pins.iter()) {
            *slot = pin.is_low();
        }
        levels
    }

    /// A newly pressed input, once the level has held steady. Holding a button
    /// does not repeat, which matters when the action is "load a preset".
    fn newly_pressed(&mut self) -> Option<usize> {
        if let Some(index) = self.pending.pop_front() {
            return Some(index);
        }
        // Active-low: the HAT pulls a pin to ground when its switch closes.
        let mut levels = [false; 8];
        for (slot, pin) in levels.iter_mut().zip(self.pins.iter()) {
            *slot = pin.is_low();
        }
        self.pending.extend(self.debounce.sample(levels));
        self.pending.pop_front()
    }
}

impl HatButtons {
    /// Wait for any input and report *which* one, rather than what it means.
    ///
    /// For diagnostics: `button_test` needs to know that KEY3 specifically
    /// registered, not merely that something asked to quit.
    pub fn poll_raw(&mut self, timeout: Duration) -> Option<usize> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(index) = self.newly_pressed() {
                return Some(index);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl InputSource for HatButtons {
    fn poll(&mut self, timeout: Duration) -> Option<InputEvent> {
        // GPIO reads do not block, so poll across the timeout rather than
        // returning instantly and spinning the caller's loop.
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(index) = self.newly_pressed() {
                return Some(BINDINGS[index]);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
