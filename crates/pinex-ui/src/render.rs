//! Rendering seam.
//!
//! The Pi drives an SPI panel; a laptop drives a terminal; a test drives a
//! `Vec`. Only [`Renderer`] knows the difference, which is what keeps the
//! browser logic testable with no display attached.

use crate::browser::View;

pub trait Renderer {
    fn render(&mut self, view: &View<'_>);

    /// Redraw only what moves between animation frames.
    ///
    /// Defaults to a full redraw, which is always correct; implementations that
    /// pay for pixels — a real panel over SPI — override it.
    fn render_scroll(&mut self, view: &View<'_>) {
        self.render(view);
    }
}

/// Formats a view as the lines a small display would show.
///
/// Shared by the terminal renderer and the eventual SPI panel so both show the
/// same thing, and so what the panel *would* show is assertable in a test.
///
/// # This is also the redraw key
///
/// `App::step` repaints only when these lines change, so **anything the panel
/// draws must be represented here**. A field the panel renders but this
/// summary ignores does not merely go unlogged — the screen stops following it
/// altogether, while every other layer works perfectly.
///
/// The stomp LED shipped dead exactly this way: the pedal reported bypass, the
/// browser stored it, `panel::draw` drew it, and the app never called `render`
/// because the summary was byte-identical either way. `set_theme` had already
/// had to work around the same thing by nulling `last_frame` by hand.
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

    // Bypassed belongs on the "now playing" line because it is the truth about
    // what is being heard: the preset is still loaded, and none of it is
    // reaching the amp.
    let bypassed = if view.bypassed { "  BYPASSED" } else { "" };
    out.push(match (view.active, view.active_name) {
        (Some(i), Some(name)) => format!("NOW  {:02} {name}{bypassed}", i + 1),
        (Some(i), None) => format!("NOW  {:02}{bypassed}", i + 1),
        (None, _) => format!("NOW  --{bypassed}"),
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

    fn render_scroll(&mut self, view: &View<'_>) {
        for renderer in self.0.iter_mut() {
            renderer.render_scroll(view);
        }
    }
}

/// Prints to stdout. Used when running on a laptop with no panel.
///
/// Dedupes internally: the app redraws whenever a name is scrolling, which is
/// several times a second, and none of that belongs in a log.
#[derive(Debug, Default)]
pub struct ConsoleRenderer {
    last: Option<Vec<String>>,
}

impl Renderer for ConsoleRenderer {
    fn render(&mut self, view: &View<'_>) {
        let current = lines(view);
        if self.last.as_ref() == Some(&current) {
            return;
        }
        println!("{}", current.join("  |  "));
        self.last = Some(current);
    }

    /// Nothing to say: a scroll does not change the text summary.
    fn render_scroll(&mut self, _view: &View<'_>) {}
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

    /// `lines` is not only what the console prints — it is the key the app
    /// compares to decide whether to repaint the panel. Anything the panel
    /// draws that is missing here is invisible to that comparison, so the
    /// screen silently stops following it.
    ///
    /// That is exactly how the stomp LED shipped dead: the bypass state
    /// arrived, the browser stored it, `panel::draw` drew it, and the app never
    /// called `render` because the summary was byte-identical either way.
    #[test]
    fn a_change_the_panel_draws_must_change_the_frame_summary() {
        let connected = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let engaged = crate::browser::View {
            screen: crate::browser::Screen::Stomp,
            stomp_mode: true,
            bypassed: false,
            ..crate::browser::View::stub(&connected)
        };
        let bypassed = crate::browser::View {
            bypassed: true,
            ..engaged.clone()
        };

        assert_ne!(
            lines(&engaged),
            lines(&bypassed),
            "the panel draws the bypass state, so the redraw key must see it"
        );
    }

    fn view_of<'a>(
        connection: &'a Connection,
        active: Option<u8>,
        name: Option<&'a str>,
    ) -> View<'a> {
        View {
            cursor_name: name,
            active,
            active_name: name,
            ..View::stub(connection)
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
