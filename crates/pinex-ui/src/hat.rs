//! The Waveshare 1.44" LCD HAT: an ST7735S panel plus a joystick and three keys.
//!
//! This is the only file in the workspace that talks to SPI or GPIO, and it is
//! deliberately thin — open the bus, initialise the controller, hand a
//! `DrawTarget` to [`crate::panel::draw`]. Everything about *what* is drawn
//! lives in `panel.rs`, where it is tested against an in-memory buffer.
//!
//! Pin assignments are the vendor's, taken from the HAT's own demo code:
//!
//! | Signal | BCM | | Signal | BCM |
//! |---|---|---|---|---|
//! | CS | 8 (CE0) | | Joystick up/down | 6 / 19 |
//! | DC | 25 | | Joystick left/right | 5 / 26 |
//! | RST | 27 | | Joystick press | 13 |
//! | Backlight | 24 | | KEY1/2/3 | 21 / 20 / 16 |
//!
//! Built only with `--features hat`, because `rppal` is Linux-only.

use mipidsi::interface::SpiInterface;
use mipidsi::models::ST7735s;
use mipidsi::options::{ColorInversion, Orientation, Rotation};
use mipidsi::{Builder, Display};
use rppal::gpio::{Gpio, OutputPin};
use rppal::hal::Delay;
use rppal::spi::{Bus, Mode, SimpleHalSpiDevice, SlaveSelect, Spi};

use crate::browser::View;
use crate::panel;
use crate::render::Renderer;

/// BCM pin numbers, from the vendor demo code.
pub mod pins {
    pub const DC: u8 = 25;
    pub const RST: u8 = 27;
    pub const BACKLIGHT: u8 = 24;

    pub const JOY_UP: u8 = 6;
    pub const JOY_DOWN: u8 = 19;
    pub const JOY_LEFT: u8 = 5;
    pub const JOY_RIGHT: u8 = 26;
    pub const JOY_PRESS: u8 = 13;
    pub const KEY1: u8 = 21;
    pub const KEY2: u8 = 20;
    pub const KEY3: u8 = 16;
}

/// `PINEX_PANEL_ROTATION` = 0 | 90 | 180 | 270. Which way up the glass reads
/// depends on how the unit is mounted, so it is configuration, not a constant.
fn rotation_from_env() -> Rotation {
    match std::env::var("PINEX_PANEL_ROTATION").as_deref() {
        Ok("0") => Rotation::Deg0,
        Ok("180") => Rotation::Deg180,
        Ok("270") => Rotation::Deg270,
        _ => Rotation::Deg90,
    }
}

/// Where the visible 128×128 window sits inside the controller's 132×162 RAM.
///
/// Waveshare's driver uses (1, 2) in the panel's native orientation. Rotating
/// by a quarter turn swaps the axes, so the offset must swap with them —
/// otherwise the noise band simply moves to a different edge, which reads as
/// "the fix didn't work" rather than "the offset is now on the wrong axis".
fn default_offset(rotation: Rotation) -> (u16, u16) {
    match rotation {
        Rotation::Deg0 | Rotation::Deg180 => (1, 2),
        Rotation::Deg90 | Rotation::Deg270 => (2, 1),
    }
}

/// `PINEX_PANEL_OFFSET` = "x,y", overriding the derived default.
fn offset_from_env(rotation: Rotation) -> (u16, u16) {
    let Ok(raw) = std::env::var("PINEX_PANEL_OFFSET") else {
        return default_offset(rotation);
    };
    let mut parts = raw.split(',').map(str::trim).map(str::parse::<u16>);
    match (parts.next(), parts.next()) {
        (Some(Ok(x)), Some(Ok(y))) => (x, y),
        _ => default_offset(rotation),
    }
}

/// The panel's SPI clock. The ST7735S datasheet allows more, but a HAT sits on
/// unshielded header pins; 32 MHz redraws a 128×128 frame in a few ms and has
/// margin to spare.
const SPI_CLOCK_HZ: u32 = 32_000_000;

#[derive(Debug)]
pub enum HatError {
    Gpio(rppal::gpio::Error),
    Spi(rppal::spi::Error),
    /// The controller rejected initialisation.
    Init(String),
}

impl std::fmt::Display for HatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gpio(e) => write!(f, "GPIO: {e}"),
            Self::Spi(e) => write!(
                f,
                "SPI: {e} (is SPI enabled? `dtparam=spi=on` in config.txt, then reboot)"
            ),
            Self::Init(e) => write!(f, "display init: {e}"),
        }
    }
}

impl std::error::Error for HatError {}

impl From<rppal::gpio::Error> for HatError {
    fn from(e: rppal::gpio::Error) -> Self {
        Self::Gpio(e)
    }
}

impl From<rppal::spi::Error> for HatError {
    fn from(e: rppal::spi::Error) -> Self {
        Self::Spi(e)
    }
}

/// Bytes per SPI transfer.
///
/// **Capped by the kernel, not by us.** Linux's `spidev` refuses a transfer
/// larger than its `bufsiz` module parameter, which defaults to 4096; asking
/// for more fails at draw time with `EMSGSIZE` ("Message too long") rather than
/// at open, so it looks like a working display that never paints.
///
/// A full 128×128 RGB565 frame is 32 KiB, so a redraw is several transfers.
/// That is fine — it is still a handful of syscalls per frame. Raising the
/// limit would mean a boot-config change on every machine this runs on, which
/// is a worse trade than a few extra writes.
const TRANSFER_BUFFER_BYTES: usize = 4096;

pub type Panel = Display<SpiInterface<'static, SimpleHalSpiDevice, OutputPin>, ST7735s, OutputPin>;

/// The HAT's screen, as a [`Renderer`].
pub struct HatDisplay {
    panel: Panel,
    backlight: OutputPin,
}

impl HatDisplay {
    /// Open SPI, reset the controller and clear the screen.
    pub fn open() -> Result<Self, HatError> {
        let gpio = Gpio::new()?;
        let dc = gpio.get(pins::DC)?.into_output();
        let rst = gpio.get(pins::RST)?.into_output();
        let mut backlight = gpio.get(pins::BACKLIGHT)?.into_output();

        let spi = Spi::new(Bus::Spi0, SlaveSelect::Ss0, SPI_CLOCK_HZ, Mode::Mode0)?;
        let device = SimpleHalSpiDevice::new(spi);
        // Leaked deliberately: the interface borrows this for as long as the
        // display exists, and the display lives for the life of the process.
        // One allocation, never freed, rather than a self-referential struct.
        let buffer: &'static mut [u8] =
            Box::leak(vec![0u8; TRANSFER_BUFFER_BYTES].into_boxed_slice());
        let interface = SpiInterface::new(device, dc, buffer);

        let rotation = rotation_from_env();
        let (x_offset, y_offset) = offset_from_env(rotation);
        let mut delay = Delay::new();
        let panel = Builder::new(ST7735s, interface)
            .reset_pin(rst)
            .orientation(Orientation::new().rotate(rotation))
            .invert_colors(ColorInversion::Normal)
            .display_size(panel::WIDTH as u16, panel::HEIGHT as u16)
            // The ST7735S has 132x162 of RAM but only 128x128 is visible, so
            // the window sits at an offset. Without it the panel shows a band
            // of uninitialised controller RAM along one edge — which looks like
            // noise, not like a configuration mistake.
            .display_offset(x_offset, y_offset)
            .init(&mut delay)
            .map_err(|e| HatError::Init(format!("{e:?}")))?;

        // Backlight last: a lit panel showing uninitialised memory is a worse
        // first impression than a dark one.
        backlight.set_high();

        Ok(Self { panel, backlight })
    }

    /// Turn the backlight off without tearing down the panel.
    pub fn backlight(&mut self, on: bool) {
        if on {
            self.backlight.set_high();
        } else {
            self.backlight.set_low();
        }
    }

    /// Draw anything onto the panel.
    ///
    /// Exists so calibration and diagnostics can use the same initialised
    /// display without `panel` having to know about them.
    pub fn with_target<F, E>(&mut self, draw: F) -> Result<(), String>
    where
        F: FnOnce(&mut Panel) -> Result<(), E>,
        E: std::fmt::Debug,
    {
        draw(&mut self.panel).map_err(|e| format!("{e:?}"))
    }

    /// Draw a view, reporting rather than swallowing a bus failure.
    pub fn show(&mut self, view: &View<'_>) -> Result<(), String> {
        panel::draw(&mut self.panel, view).map_err(|e| format!("{e:?}"))
    }
}

impl Renderer for HatDisplay {
    fn render(&mut self, view: &View<'_>) {
        // A renderer cannot fail upward, and a dead panel must not stop the
        // pedal working — the web page and the pedal itself carry on.
        if let Err(e) = self.show(view) {
            eprintln!("! panel: {e}");
        }
    }

    fn render_scroll(&mut self, view: &View<'_>) {
        if let Err(e) = panel::draw_scroll(&mut self.panel, view).map_err(|e| format!("{e:?}")) {
            eprintln!("! panel: {e}");
        }
    }
}
