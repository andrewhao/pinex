//! The preset browser: a pure state machine.
//!
//! Takes [`PedalEvent`]s and user [`InputEvent`]s in, gives [`Command`]s and a
//! renderable [`View`] out. No I/O, no threads, no hardware — which is what
//! makes every rule below testable without a pedal attached.
//!
//! # The rule that shapes everything
//!
//! **The pedal is the source of truth.** Pressing Select does not change which
//! preset is active; it emits a request. `active` moves only when the pedal
//! reports a new state. That is what stops the display from claiming a preset
//! the pedal never loaded — a display that lies on stage is worse than one that
//! lags.
//!
//! The browsing cursor is separate from the active preset for the same reason:
//! the player can look at preset 7 while still hearing preset 3.

use pinex_device::{Command, PedalEvent};
pub use pinex_input::InputEvent;
use pinex_proto::state::MAX_PRESETS;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Connection {
    #[default]
    Disconnected,
    Connected {
        firmware: String,
    },
}

impl Connection {
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }
}

/// A read-only snapshot for a renderer.
#[derive(Debug, Clone, PartialEq)]
pub struct View<'a> {
    pub connection: &'a Connection,
    /// 0-based index the player is browsing.
    pub cursor: u8,
    pub cursor_name: Option<&'a str>,
    /// 0-based index the pedal says it is playing, if known.
    pub active: Option<u8>,
    pub active_name: Option<&'a str>,
    /// True while a Select has been sent but the pedal has not confirmed it.
    pub pending: bool,
    /// Most recent parse failure, surfaced rather than swallowed.
    pub last_error: Option<&'a str>,
}

impl View<'_> {
    /// What a one-line display should show for the cursor.
    pub fn cursor_label(&self) -> String {
        match self.cursor_name {
            Some(name) => format!("{:02} {name}", self.cursor + 1),
            None => format!("{:02} ...", self.cursor + 1),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PresetBrowser {
    connection: Connection,
    names: Vec<Option<String>>,
    /// Per-preset RGB, as the pedal lights its own ring. Read from the state.
    colors: Vec<[u8; 3]>,
    cursor: u8,
    active: Option<u8>,
    pending: Option<u8>,
    last_error: Option<String>,
}

impl PresetBrowser {
    pub fn new() -> Self {
        Self {
            names: vec![None; MAX_PRESETS as usize],
            ..Default::default()
        }
    }

    pub fn view(&self) -> View<'_> {
        View {
            connection: &self.connection,
            cursor: self.cursor,
            cursor_name: self.name_of(self.cursor),
            active: self.active,
            active_name: self.active.and_then(|i| self.name_of(i)),
            pending: self.pending.is_some(),
            last_error: self.last_error.as_deref(),
        }
    }

    /// The pedal's own colour for `index`, once a state has arrived.
    pub fn color_at(&self, index: u8) -> Option<[u8; 3]> {
        self.colors.get(index as usize).copied()
    }

    /// The name known for `index`, if the pedal has reported it.
    pub fn name_at(&self, index: u8) -> Option<&str> {
        self.name_of(index)
    }

    fn name_of(&self, index: u8) -> Option<&str> {
        self.names.get(index as usize)?.as_deref()
    }

    /// React to something the pedal said.
    pub fn apply(&mut self, event: &PedalEvent) -> Vec<Command> {
        match event {
            PedalEvent::Connected { firmware } => {
                self.connection = Connection::Connected {
                    firmware: firmware.clone(),
                };
                self.last_error = None;
                self.sync_all()
            }
            PedalEvent::Disconnected => {
                // Keep the names — they are still true, and re-fetching 20
                // presets on every USB blip would be needless traffic. Drop
                // `active`, because we no longer know what is playing, and
                // showing a stale "now playing" is exactly the lie to avoid.
                self.connection = Connection::Disconnected;
                self.active = None;
                self.pending = None;
                Vec::new()
            }
            PedalEvent::StateChanged(state) => {
                // The pedal lights each preset a colour; mirror it so the
                // display can agree with the hardware rather than invent its own.
                if let Ok(colors) = state.preset_colors() {
                    self.colors = colors;
                }
                match state.active_preset() {
                    Ok(active) => {
                        let first_sync = self.active.is_none();
                        self.active = Some(active);
                        // Clear the pending flag only when the pedal confirms
                        // the preset we actually asked for.
                        if self.pending == Some(active) {
                            self.pending = None;
                        }
                        // On the very first state, park the cursor on what is
                        // playing so the display opens on the truth.
                        if first_sync {
                            self.cursor = active.min(MAX_PRESETS - 1);
                        }
                    }
                    Err(e) => self.last_error = Some(e.to_string()),
                }
                Vec::new()
            }
            PedalEvent::PresetName(info) => {
                if let Some(slot) = self.names.get_mut(info.index as usize) {
                    *slot = Some(info.name.clone());
                }
                Vec::new()
            }
            // Deliberately does not clear `pending`: the pedal acknowledges
            // writes it then reverts, so treating this as confirmation would
            // make the display claim a preset change that never happened.
            PedalEvent::WriteAcknowledged => Vec::new(),
            PedalEvent::ParseError { reason, .. } => {
                self.last_error = Some(reason.clone());
                Vec::new()
            }
        }
    }

    /// React to something the player did.
    pub fn handle(&mut self, input: InputEvent) -> Vec<Command> {
        match input {
            InputEvent::Next => {
                self.cursor = (self.cursor + 1) % MAX_PRESETS;
                Vec::new()
            }
            InputEvent::Prev => {
                self.cursor = (self.cursor + MAX_PRESETS - 1) % MAX_PRESETS;
                Vec::new()
            }
            InputEvent::Select => {
                // Deliberately does NOT set `active`. See the module docs.
                if !self.connection.is_connected() {
                    return Vec::new();
                }
                self.pending = Some(self.cursor);
                vec![Command::SetPreset(self.cursor)]
            }
            InputEvent::Refresh => self.sync_all(),
            // Shutdown is the app loop's business, not the browser's.
            InputEvent::Quit => Vec::new(),
        }
    }

    /// Ask for the state and every preset name.
    fn sync_all(&mut self) -> Vec<Command> {
        let mut out = vec![Command::RequestState];
        out.extend((0..MAX_PRESETS).map(Command::RequestPreset));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pinex_proto::message::PresetInfo;
    use pinex_proto::state::{PedalState, Slot};

    fn named(index: u8, name: &str) -> PedalEvent {
        PedalEvent::PresetName(PresetInfo {
            index,
            name: name.to_string(),
        })
    }

    /// A state body with `active` loaded in slot A and slot A selected.
    fn state_with_active(preset: u8) -> PedalEvent {
        let mut raw = vec![0u8; 64];
        let len = raw.len();
        raw[len - pinex_proto::state::offset_from_end::CURRENT_SLOT] = Slot::A as u8;
        raw[len - pinex_proto::state::offset_from_end::SLOT_A_PRESET] = preset;
        PedalEvent::StateChanged(PedalState::from_body(raw).unwrap())
    }

    fn connected() -> PresetBrowser {
        let mut b = PresetBrowser::new();
        b.apply(&PedalEvent::Connected {
            firmware: "1.3.17".into(),
        });
        b
    }

    #[test]
    fn connecting_asks_for_the_state_and_every_preset_name() {
        let mut b = PresetBrowser::new();
        let cmds = b.apply(&PedalEvent::Connected {
            firmware: "1.3.17".into(),
        });

        assert_eq!(cmds[0], Command::RequestState);
        assert_eq!(cmds.len(), 1 + MAX_PRESETS as usize);
        for i in 0..MAX_PRESETS {
            assert!(cmds.contains(&Command::RequestPreset(i)), "missing {i}");
        }
        assert!(b.view().connection.is_connected());
    }

    #[test]
    fn preset_names_land_at_their_index() {
        let mut b = connected();
        b.apply(&named(0, "TF BENSON PREAMP - 1"));
        b.apply(&named(15, "TF TILT - 1 ADV"));

        assert_eq!(b.view().cursor_name, Some("TF BENSON PREAMP - 1"));
        for _ in 0..15 {
            b.handle(InputEvent::Next);
        }
        assert_eq!(b.view().cursor_name, Some("TF TILT - 1 ADV"));
    }

    #[test]
    fn the_cursor_wraps_in_both_directions() {
        let mut b = connected();
        assert_eq!(b.view().cursor, 0);

        b.handle(InputEvent::Prev);
        assert_eq!(b.view().cursor, MAX_PRESETS - 1, "0 must wrap back to 20");

        b.handle(InputEvent::Next);
        assert_eq!(b.view().cursor, 0, "20 must wrap forward to 1");
    }

    /// The rule the whole design rests on.
    #[test]
    fn selecting_a_preset_does_not_claim_it_is_active() {
        let mut b = connected();
        b.apply(&state_with_active(3));
        b.handle(InputEvent::Next);
        b.handle(InputEvent::Next);

        let cmds = b.handle(InputEvent::Select);

        assert_eq!(cmds, vec![Command::SetPreset(5)]);
        assert_eq!(
            b.view().active,
            Some(3),
            "active must not move until the pedal says so"
        );
        assert!(b.view().pending, "the request should be visibly in flight");
    }

    #[test]
    fn the_pedal_confirming_the_request_clears_the_pending_flag() {
        let mut b = connected();
        b.apply(&state_with_active(3));
        b.handle(InputEvent::Next);
        b.handle(InputEvent::Select);
        assert!(b.view().pending);

        b.apply(&state_with_active(4));

        assert_eq!(b.view().active, Some(4));
        assert!(!b.view().pending);
    }

    /// If the pedal reports a preset we did not ask for, the request is still
    /// outstanding — someone turned a knob, or our write did not take.
    #[test]
    fn a_different_preset_arriving_leaves_the_request_pending() {
        let mut b = connected();
        b.apply(&state_with_active(3));
        b.handle(InputEvent::Next);
        b.handle(InputEvent::Select); // asks for 4

        b.apply(&state_with_active(9)); // pedal says 9

        assert_eq!(b.view().active, Some(9), "the pedal is always believed");
        assert!(b.view().pending, "our request is still unconfirmed");
    }

    /// Someone changing preset on the pedal itself must move the display.
    #[test]
    fn an_unsolicited_state_change_moves_the_active_preset() {
        let mut b = connected();
        b.apply(&state_with_active(3));
        b.apply(&state_with_active(11));
        assert_eq!(b.view().active, Some(11));
    }

    #[test]
    fn the_first_state_parks_the_cursor_on_what_is_playing() {
        let mut b = connected();
        b.apply(&state_with_active(7));
        assert_eq!(b.view().cursor, 7);
    }

    /// ...but only the first, or the cursor would fight the player's browsing.
    #[test]
    fn later_state_changes_leave_the_browsing_cursor_alone() {
        let mut b = connected();
        b.apply(&state_with_active(7));
        b.handle(InputEvent::Next);
        b.handle(InputEvent::Next);
        assert_eq!(b.view().cursor, 9);

        b.apply(&state_with_active(2));

        assert_eq!(b.view().cursor, 9, "the player is still browsing");
        assert_eq!(b.view().active, Some(2));
    }

    #[test]
    fn disconnecting_drops_the_active_preset_but_keeps_the_names() {
        let mut b = connected();
        b.apply(&named(4, "TF TILT - 1 ADV"));
        b.apply(&state_with_active(4));

        b.apply(&PedalEvent::Disconnected);

        let view = b.view();
        assert!(!view.connection.is_connected());
        assert_eq!(view.active, None, "must not claim to know what is playing");
        assert_eq!(
            view.cursor_name,
            Some("TF TILT - 1 ADV"),
            "names still true"
        );
    }

    #[test]
    fn selecting_while_disconnected_sends_nothing() {
        let mut b = PresetBrowser::new();
        assert_eq!(b.handle(InputEvent::Select), Vec::new());
        assert!(!b.view().pending);
    }

    /// The pedal lights each preset a colour; the display should agree with the
    /// hardware rather than invent its own palette.
    #[test]
    fn preset_colours_arrive_with_the_state() {
        let mut b = connected();
        assert_eq!(b.color_at(0), None, "unknown until a state arrives");

        // A body carrying a well-formed 20-entry colour array.
        let mut raw = vec![0u8; 32];
        raw.push(0xBA);
        raw.push(MAX_PRESETS);
        for i in 0..MAX_PRESETS {
            raw.extend_from_slice(&[0xB9, 0x03, i, 0x80, 0xFF, 0x00]);
        }
        raw.extend_from_slice(&[0u8; 32]);
        let len = raw.len();
        raw[len - pinex_proto::state::offset_from_end::CURRENT_SLOT] = Slot::A as u8;
        raw[len - pinex_proto::state::offset_from_end::SLOT_A_PRESET] = 0;

        b.apply(&PedalEvent::StateChanged(
            PedalState::from_body(raw).unwrap(),
        ));

        assert_eq!(b.color_at(0), Some([0, 255, 0]));
        assert_eq!(b.color_at(5), Some([5, 255, 0]));
        assert_eq!(b.color_at(19), Some([19, 255, 0]));
        assert_eq!(b.color_at(20), None, "no colour beyond the preset range");
    }

    #[test]
    fn parse_errors_are_surfaced_not_swallowed() {
        let mut b = connected();
        b.apply(&PedalEvent::ParseError {
            raw: vec![0xde, 0xad],
            reason: "unrecognised message type 0x9999".into(),
        });
        assert_eq!(
            b.view().last_error,
            Some("unrecognised message type 0x9999")
        );
    }

    #[test]
    fn the_display_label_is_one_based_because_players_count_from_one() {
        let mut b = connected();
        b.apply(&named(0, "TF BENSON PREAMP - 1"));
        assert_eq!(b.view().cursor_label(), "01 TF BENSON PREAMP - 1");

        b.handle(InputEvent::Prev);
        assert_eq!(
            b.view().cursor_label(),
            "20 ...",
            "unknown names show as ..."
        );
    }

    /// A preset index outside the pedal's range must not panic or write past
    /// the array — this is untrusted input from a device we do not control.
    #[test]
    fn an_out_of_range_preset_name_is_ignored_rather_than_panicking() {
        let mut b = connected();
        b.apply(&named(200, "impossible"));
        assert_eq!(b.view().cursor_name, None);
    }
}
