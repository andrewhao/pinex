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

/// Everything about a [`View`] that changes the picture, owned and comparable.
///
/// # Why this exists rather than comparing the text summary
///
/// `App::step` repaints only when something changed, and it used to decide that
/// by diffing [`lines`]. That made the console summary silently load-bearing:
/// any field the panel drew but the summary omitted stopped being followed on
/// screen, while every other layer — pedal, parser, browser, `panel::draw` —
/// worked perfectly. It is a maddening bug to find, because a unit test calling
/// `panel::draw` directly passes, that being exactly the call the app skips.
///
/// It happened three times: the theme (worked around by hand with
/// `last_frame = None`), the stomp bypass LED, and finally the whole Levels
/// page, which was invisible because paging to it changed nothing in the
/// summary.
///
/// So the key is built by **exhaustively destructuring `View` with no `..`**.
/// Add a field to `View` and this stops compiling until someone decides whether
/// it affects what is drawn. That is a compiler error instead of a fourth
/// silent recurrence.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderKey {
    connected: Option<String>,
    cursor: u8,
    cursor_name: Option<String>,
    cursor_color: Option<[u8; 3]>,
    active: Option<u8>,
    active_name: Option<String>,
    pending: bool,
    last_error: Option<String>,
    screen: crate::browser::Screen,
    selected: pinex_proto::state::Slot,
    slot_presets: Option<[u8; 3]>,
    active_slot: Option<pinex_proto::state::Slot>,
    stomp_mode: bool,
    bypassed: bool,
    /// Floats as bits, so NaN compares equal to itself and cannot force a
    /// repaint on every single tick.
    gain_db: u32,
    level_focus: crate::browser::Level,
    master_volume_db: Option<u32>,
    slot_names: [Option<String>; 3],
    slot_colors: [Option<[u8; 3]>; 3],
}

impl RenderKey {
    pub fn of(view: &View<'_>) -> Self {
        // Exhaustive on purpose — no `..`. See the type docs.
        let View {
            connection,
            cursor,
            cursor_name,
            cursor_color,
            active,
            active_name,
            pending,
            last_error,
            screen,
            selected,
            slot_presets,
            active_slot,
            stomp_mode,
            bypassed,
            gain_db,
            level_focus,
            master_volume_db,
            slot_names,
            slot_colors,
            tick,
        } = view;

        // `tick` is deliberately excluded. It advances several times a second
        // and is what *drives* scrolling; including it would make every frame
        // look changed and turn the cheap scroll path into a full repaint —
        // the flicker this panel already had once.
        let _ = tick;

        Self {
            connected: match connection {
                crate::browser::Connection::Connected { firmware } => Some(firmware.clone()),
                crate::browser::Connection::Disconnected => None,
            },
            cursor: *cursor,
            cursor_name: cursor_name.map(str::to_string),
            cursor_color: *cursor_color,
            active: *active,
            active_name: active_name.map(str::to_string),
            pending: *pending,
            last_error: last_error.map(str::to_string),
            screen: *screen,
            selected: *selected,
            slot_presets: *slot_presets,
            active_slot: *active_slot,
            stomp_mode: *stomp_mode,
            bypassed: *bypassed,
            gain_db: gain_db.to_bits(),
            level_focus: *level_focus,
            master_volume_db: master_volume_db.map(f32::to_bits),
            slot_names: slot_names.map(|n| n.map(str::to_string)),
            slot_colors: *slot_colors,
        }
    }
}

/// Formats a view as the lines a small display would show.
///
/// Shared by the terminal renderer and the eventual SPI panel so both show the
/// same thing, and so what the panel *would* show is assertable in a test.
///
/// This is the human-readable summary and **not** the redraw key — that is
/// [`RenderKey`], which is derived from the whole view. Keeping the two apart
/// is deliberate: while this doubled as the repaint trigger, every field it
/// happened not to mention silently stopped being drawn.
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

    // Both levels, always — not only on the page that shows them. This line is
    // the redraw key, and a value that moves without changing it is a value the
    // panel silently stops following. It is also worth having in the journal:
    // "the output was at -40" is the first thing to check when a gig went quiet.
    out.push(match view.master_volume_db {
        Some(db) => format!("VOL {db:+.1} dB   TRIM {:+.1} dB", view.gain_db),
        None => format!("VOL --      TRIM {:+.1} dB", view.gain_db),
    });

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

    /// The same trap as the bypass LED, and the Levels page already had it:
    /// nothing about either level appeared in the redraw key, so turning one
    /// changed the numbers the panel draws while the panel was never asked to
    /// draw them.
    #[test]
    fn turning_either_level_changes_the_frame_summary() {
        let connected = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let base = crate::browser::View {
            screen: crate::browser::Screen::Levels,
            gain_db: 0.0,
            master_volume_db: Some(-12.0),
            ..crate::browser::View::stub(&connected)
        };

        let louder = crate::browser::View {
            master_volume_db: Some(-11.0),
            ..base.clone()
        };
        assert_ne!(
            RenderKey::of(&base),
            RenderKey::of(&louder),
            "the panel draws the output level, so the redraw key must see it"
        );

        let trimmed = crate::browser::View {
            gain_db: 1.5,
            ..base.clone()
        };
        assert_ne!(
            RenderKey::of(&base),
            RenderKey::of(&trimmed),
            "the panel draws the input trim, so the redraw key must see it"
        );
    }

    /// The bug that made the Levels page invisible: paging to it changed
    /// nothing the redraw key could see, so the panel carried on drawing the
    /// previous page while the browser really had moved.
    #[test]
    fn changing_page_must_change_the_redraw_key() {
        let connected = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let stomp = crate::browser::View {
            screen: crate::browser::Screen::Stomp,
            ..crate::browser::View::stub(&connected)
        };
        let levels = crate::browser::View {
            screen: crate::browser::Screen::Levels,
            ..stomp.clone()
        };
        assert_ne!(
            RenderKey::of(&stomp),
            RenderKey::of(&levels),
            "the page must be visible to the redraw key"
        );
    }

    /// Which row the Levels page is editing is drawn, so it must repaint.
    #[test]
    fn moving_the_level_focus_changes_the_redraw_key() {
        let connected = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let vol = crate::browser::View {
            screen: crate::browser::Screen::Levels,
            level_focus: crate::browser::Level::Volume,
            ..crate::browser::View::stub(&connected)
        };
        let trim = crate::browser::View {
            level_focus: crate::browser::Level::Trim,
            ..vol.clone()
        };
        assert_ne!(RenderKey::of(&vol), RenderKey::of(&trim));
    }

    /// ...but the animation clock must **not**. It advances several times a
    /// second; counting it as a change would turn the cheap scroll path into a
    /// full clear-and-repaint, which is the flicker this panel already had.
    #[test]
    fn the_scroll_clock_alone_does_not_force_a_repaint() {
        let connected = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        let first = crate::browser::View {
            tick: 0,
            ..crate::browser::View::stub(&connected)
        };
        let later = crate::browser::View {
            tick: 900,
            ..first.clone()
        };
        assert_eq!(
            RenderKey::of(&first),
            RenderKey::of(&later),
            "only the scroll clock moved; a full repaint here is the flicker bug"
        );
    }

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
            RenderKey::of(&engaged),
            RenderKey::of(&bypassed),
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
