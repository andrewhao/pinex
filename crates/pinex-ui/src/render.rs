//! Rendering seam.
//!
//! The Pi drives an SPI panel; a laptop drives a terminal; a test drives a
//! `Vec`. Only [`Renderer`] knows the difference, which is what keeps the
//! browser logic testable with no display attached.

use crate::browser::View;

pub trait Renderer {
    fn render(&mut self, view: &View<'_>);
}

/// Formats a view as the lines a small display would show.
///
/// Shared by the terminal renderer and the eventual SPI panel so both show the
/// same thing, and so what the panel *would* show is assertable in a test.
pub fn lines(view: &View<'_>) -> Vec<String> {
    let mut out = Vec::new();

    match view.connection {
        crate::browser::Connection::Disconnected => {
            // The spec calls for this to be explicit and unmistakable.
            out.push("NO PEDAL".to_string());
        }
        crate::browser::Connection::Connected { firmware } => {
            out.push(format!("Tonex ONE  fw {firmware}"));
        }
    }

    out.push(match (view.active, view.active_name) {
        (Some(i), Some(name)) => format!("NOW  {:02} {name}", i + 1),
        (Some(i), None) => format!("NOW  {:02}", i + 1),
        (None, _) => "NOW  --".to_string(),
    });

    let marker = if Some(view.cursor) == view.active {
        "*"
    } else {
        " "
    };
    let pending = if view.pending { "  (sending...)" } else { "" };
    out.push(format!("{marker}{}{pending}", view.cursor_label()));

    if let Some(err) = view.last_error {
        out.push(format!("! {err}"));
    }
    out
}

/// Draws to several renderers at once.
///
/// The Pi runs a panel *and* logs to the journal; both should show the same
/// thing, and neither should be special-cased in the app loop.
#[derive(Default)]
pub struct Multi(pub Vec<Box<dyn Renderer + Send>>);

impl Multi {
    pub fn with(mut self, renderer: impl Renderer + Send + 'static) -> Self {
        self.0.push(Box::new(renderer));
        self
    }
}

impl Renderer for Multi {
    fn render(&mut self, view: &View<'_>) {
        for renderer in self.0.iter_mut() {
            renderer.render(view);
        }
    }
}

/// Prints to stdout. Used when running on a laptop with no panel.
#[derive(Debug, Default)]
pub struct ConsoleRenderer;

impl Renderer for ConsoleRenderer {
    fn render(&mut self, view: &View<'_>) {
        println!("{}", lines(view).join("  |  "));
    }
}

/// Keeps every frame it was given, so tests can assert what was displayed.
#[derive(Debug, Default)]
pub struct RecordingRenderer {
    pub frames: Vec<Vec<String>>,
}

impl RecordingRenderer {
    pub fn last(&self) -> Option<&Vec<String>> {
        self.frames.last()
    }
}

impl Renderer for RecordingRenderer {
    fn render(&mut self, view: &View<'_>) {
        self.frames.push(lines(view));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::Connection;

    fn view_of<'a>(
        connection: &'a Connection,
        active: Option<u8>,
        name: Option<&'a str>,
    ) -> View<'a> {
        View {
            connection,
            cursor: 0,
            cursor_name: name,
            cursor_color: None,
            active,
            active_name: name,
            pending: false,
            last_error: None,
        }
    }

    #[test]
    fn a_disconnected_pedal_says_so_explicitly() {
        let c = Connection::Disconnected;
        let rendered = lines(&view_of(&c, None, None));
        assert_eq!(rendered[0], "NO PEDAL");
        assert_eq!(rendered[1], "NOW  --", "must not claim a preset is playing");
    }

    #[test]
    fn a_connected_pedal_shows_firmware_and_the_active_preset() {
        let c = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let rendered = lines(&view_of(&c, Some(6), Some("TF TILT - 1 ADV")));
        assert_eq!(rendered[0], "Tonex ONE  fw 1.3.17");
        assert_eq!(rendered[1], "NOW  07 TF TILT - 1 ADV");
    }

    #[test]
    fn parse_errors_reach_the_display() {
        let c = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let mut view = view_of(&c, Some(0), Some("x"));
        view.last_error = Some("bad crc");
        assert!(lines(&view).iter().any(|l| l.contains("bad crc")));
    }
}
