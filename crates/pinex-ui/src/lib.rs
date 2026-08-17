//! Display rendering. **Not yet implemented (M2).**
//!
//! `embedded-graphics` over `mipidsi` driving an ST7789 SPI panel: large preset
//! number, preset name beneath it, connection state, and an explicit "NO PEDAL"
//! screen.
//!
//! Pin `rppal`, `mipidsi` and `display-interface-spi` to a single `embedded-hal`
//! major version when this lands; a mismatch there is expected setup work rather
//! than a surprise.

#[cfg(feature = "hat")]
pub mod hat;

pub mod browser;
pub mod panel;
pub mod preview;
pub mod render;
pub mod skin;

pub use browser::{Connection, InputEvent, Level, PresetBrowser, Screen, View};
pub use preview::{PreviewPanel, PreviewRenderer};
pub use render::{lines, ConsoleRenderer, Multi, RecordingRenderer, RenderKey, Renderer};
