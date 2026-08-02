//! Footswitch and button input. **Not yet implemented (M2/M3).**
//!
//! An `InputSource` trait with three implementations: interrupt-driven
//! `GpioInput` via `rppal`, `HatButtons` for the display HAT's onboard buttons,
//! and `StdinInput` so the whole input → state → render loop can be exercised
//! without hardware.
//!
//! Input emits `Command`s. A switch press is a *request*, never a local state
//! change — the pedal remains the source of truth, which is what stops the
//! display from ever lying.

#![allow(unused_imports)]

use pinex_proto as _;
