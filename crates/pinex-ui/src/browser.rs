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
use pinex_proto::message::{MAX_MASTER_VOLUME_DB, MIN_MASTER_VOLUME_DB};
use pinex_proto::state::{Slot, MAX_INPUT_TRIM_DB, MAX_PRESETS, MIN_INPUT_TRIM_DB};

/// Which page the panel is showing.
///
/// Three, deliberately. A stage device that needs a menu tree is a stage device
/// nobody uses under pressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    /// The two footswitch slots side by side: what is in A, what is in B.
    #[default]
    Slots,
    /// Stomp mode, where the pedal exposes a third slot.
    Stomp,
    /// Output level and input trim, together.
    Levels,
}

impl Screen {
    pub fn next(self) -> Self {
        match self {
            Self::Slots => Self::Stomp,
            Self::Stomp => Self::Levels,
            Self::Levels => Self::Slots,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Slots => "A/B",
            Self::Stomp => "STOMP",
            Self::Levels => "LEVELS",
        }
    }
}

/// Which of the two levels the Levels page is editing.
///
/// One page, two values, because a stage box with four pages is a stage box
/// nobody pages through under pressure. Left/right picks; up/down moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    /// Master volume: the output level, and the one you would ride at a venue.
    #[default]
    Volume,
    /// Input trim: gain staging, set once to match the instrument.
    Trim,
}

impl Level {
    pub fn other(self) -> Self {
        match self {
            Self::Volume => Self::Trim,
            Self::Trim => Self::Volume,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Volume => "VOL",
            Self::Trim => "TRIM",
        }
    }
}

/// How far one nudge moves the output level.
///
/// A whole decibel, not the trim's half: the output range is 43 dB wide against
/// the trim's 30, and this is the one you adjust standing up.
const VOLUME_STEP_DB: f32 = 1.0;

/// How far one nudge moves the gain.
const GAIN_STEP_DB: f32 = 0.5;

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
    /// The pedal's own colour for the browsed preset, so a panel can agree
    /// with the pedal's ring rather than invent a palette.
    pub cursor_color: Option<[u8; 3]>,
    /// 0-based index the pedal says it is playing, if known.
    pub active: Option<u8>,
    pub active_name: Option<&'a str>,
    /// True while a Select has been sent but the pedal has not confirmed it.
    pub pending: bool,
    /// Most recent parse failure, surfaced rather than swallowed.
    pub last_error: Option<&'a str>,
    /// Which page is showing.
    pub screen: Screen,
    /// The slot being edited on the Slots page.
    pub selected: Slot,
    /// What each slot holds, once a state has arrived.
    pub slot_presets: Option<[u8; 3]>,
    /// The slot the pedal is playing.
    pub active_slot: Option<Slot>,
    /// True when the pedal is in stomp mode.
    pub stomp_mode: bool,
    /// True when the footswitch has the pedal bypassed. Hardware-reported and
    /// unsolicited, so the panel follows the switch rather than guessing.
    pub bypassed: bool,
    /// Input trim in dB.
    pub gain_db: f32,
    /// Which level the Levels page is editing.
    pub level_focus: Level,
    /// Master volume in dB, once the pedal has reported it.
    ///
    /// `None` until then, and deliberately not defaulted: master volume is in
    /// no state message, so a number here that the pedal never sent would be
    /// invention. Output level is not a good thing to guess at.
    pub master_volume_db: Option<f32>,
    /// Names of what each slot holds, indexed by slot.
    pub slot_names: [Option<&'a str>; 3],
    /// The pedal's colour for what each slot holds.
    pub slot_colors: [Option<[u8; 3]>; 3],
    /// Frame counter, driving any scrolling text.
    pub tick: u32,
}

impl<'a> View<'a> {
    /// The preset a slot holds, if a state has arrived.
    pub fn slot_preset(&self, slot: Slot) -> Option<u8> {
        self.slot_presets.map(|s| s[slot as usize])
    }

    pub fn slot_name_for(&self, slot: Slot) -> Option<&'a str> {
        self.slot_names[slot as usize]
    }

    pub fn slot_color_for(&self, slot: Slot) -> Option<[u8; 3]> {
        self.slot_colors[slot as usize]
    }

    /// Whether anything on screen is mid-scroll.
    ///
    /// The app redraws on change alone otherwise, so without this a scrolling
    /// name would move once and then freeze.
    ///
    /// Only the Slots page shows the A/B names, and they are the only thing
    /// that scrolls. Answering from those names on a page that does not show
    /// them drove the app into the animation path five times a second on the
    /// Stomp page, where [`crate::panel::draw_scroll`] has no band to redraw and
    /// falls back to a full redraw — which starts by clearing the panel. The
    /// result was a steady flicker showing a picture that had not changed.
    pub fn animating(&self) -> bool {
        const SLOT_COLS: usize = 11;
        if self.screen != Screen::Slots {
            return false;
        }
        [Slot::A, Slot::B]
            .into_iter()
            .filter_map(|slot| {
                if self.selected == slot {
                    self.cursor_name
                } else {
                    self.slot_name_for(slot)
                }
            })
            .any(|name| crate::skin::scrolls(crate::skin::short_name(name), SLOT_COLS))
    }

    /// A minimal view, for tests and previews.
    ///
    /// Everything a caller does not care about defaults to "not known yet",
    /// which is also what the real view looks like before a state arrives.
    pub fn stub(connection: &'a Connection) -> Self {
        Self {
            connection,
            cursor: 0,
            cursor_name: None,
            cursor_color: None,
            active: None,
            active_name: None,
            pending: false,
            last_error: None,
            screen: Screen::Slots,
            selected: Slot::A,
            slot_presets: None,
            active_slot: None,
            stomp_mode: false,
            bypassed: false,
            gain_db: 0.0,
            level_focus: Level::Volume,
            master_volume_db: None,
            slot_names: [None; 3],
            slot_colors: [None; 3],
            tick: 0,
        }
    }
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
    screen: Screen,
    selected: Slot,
    slot_presets: Option<[u8; 3]>,
    active_slot: Option<Slot>,
    stomp_mode: bool,
    bypassed: bool,
    gain_db: f32,
    level_focus: Level,
    master_volume_db: Option<f32>,
    tick: u32,
    /// The preset whose name we are waiting for, while walking them all.
    awaiting_preset: Option<u8>,
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
            cursor_color: self.color_at(self.cursor),
            active: self.active,
            active_name: self.active.and_then(|i| self.name_of(i)),
            pending: self.pending.is_some(),
            last_error: self.last_error.as_deref(),
            screen: self.screen,
            selected: self.selected,
            slot_presets: self.slot_presets,
            active_slot: self.active_slot,
            stomp_mode: self.stomp_mode,
            bypassed: self.bypassed,
            gain_db: self.gain_db,
            level_focus: self.level_focus,
            master_volume_db: self.master_volume_db,
            slot_names: [
                self.slot_name(Slot::A),
                self.slot_name(Slot::B),
                self.slot_name(Slot::C),
            ],
            slot_colors: [
                self.slot_color(Slot::A),
                self.slot_color(Slot::B),
                self.slot_color(Slot::C),
            ],
            tick: self.tick,
        }
    }

    /// Advance the animation clock.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// The preset a slot holds.
    pub fn slot_preset(&self, slot: Slot) -> Option<u8> {
        self.slot_presets.map(|s| s[slot as usize])
    }

    /// The name of whatever a slot holds.
    pub fn slot_name(&self, slot: Slot) -> Option<&str> {
        self.slot_preset(slot).and_then(|p| self.name_of(p))
    }

    /// The pedal's colour for whatever a slot holds.
    pub fn slot_color(&self, slot: Slot) -> Option<[u8; 3]> {
        self.slot_preset(slot).and_then(|p| self.color_at(p))
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
                self.slot_presets = Some([
                    state.slot_preset(Slot::A),
                    state.slot_preset(Slot::B),
                    state.slot_preset(Slot::C),
                ]);
                self.active_slot = state.active_slot().ok();
                self.stomp_mode = state.stomp_mode().map(|m| m == 1).unwrap_or(false);
                self.bypassed = state.is_bypassed();
                if let Ok(trim) = state.input_trim() {
                    self.gain_db = trim;
                }
                // Follow the pedal into whichever mode it is actually in: a
                // page showing A/B while the pedal is in stomp mode is a lie.
                self.screen = match (self.screen, self.stomp_mode) {
                    (Screen::Slots, true) => Screen::Stomp,
                    (Screen::Stomp, false) => Screen::Slots,
                    (other, _) => other,
                };
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
                // Ask for the next one only when the one we asked for arrives.
                // Guarded by the index so an unsolicited reply cannot start a
                // second sweep running alongside the first.
                if self.awaiting_preset == Some(info.index) {
                    let next = info.index + 1;
                    if next < MAX_PRESETS {
                        self.awaiting_preset = Some(next);
                        return vec![Command::RequestPreset(next)];
                    }
                    self.awaiting_preset = None;
                }
                Vec::new()
            }
            // The only report of master volume there is — it appears in no
            // state message, so this is the source of truth for it.
            PedalEvent::ParamChanged(change) => {
                if change.is_master_volume() {
                    self.master_volume_db = Some(change.db);
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
    ///
    /// What a direction means depends on the page, but the shape is constant:
    /// **Next/Prev change a value, Select commits it.** Nothing commits by
    /// scrolling past it, because on stage a value that applies as you pass
    /// through it makes browsing dangerous.
    pub fn handle(&mut self, input: InputEvent) -> Vec<Command> {
        match input {
            // The axes follow the picture, which is why the browser decides and
            // not the binding table. On Slots the list runs down the screen and
            // A/B sit side by side; on Levels the rows stack and their bars run
            // across. A single fixed meaning per direction would be wrong on
            // one page or the other.
            InputEvent::Up => match self.screen {
                Screen::Levels => self.focus_level(Level::Volume),
                _ => self.nudge(-1),
            },
            InputEvent::Down => match self.screen {
                Screen::Levels => self.focus_level(Level::Trim),
                _ => self.nudge(1),
            },
            InputEvent::Left => match self.screen {
                Screen::Levels => self.nudge(-1),
                _ => self.select_slot(Slot::A),
            },
            InputEvent::Right => match self.screen {
                Screen::Levels => self.nudge(1),
                _ => self.select_slot(Slot::B),
            },
            InputEvent::Select => self.commit(),
            InputEvent::Refresh => self.sync_all(),
            InputEvent::Page => {
                self.screen = self.screen.next();
                // Stomp and Slots are two views of the pedal's actual mode, so
                // paging between them has to change the pedal, not just the
                // display.
                match (self.screen, self.stomp_mode) {
                    (Screen::Stomp, false) => vec![Command::SetStompMode(true)],
                    (Screen::Slots, true) => vec![Command::SetStompMode(false)],
                    _ => Vec::new(),
                }
            }
            // Shutdown is the app loop's business, not the browser's.
            InputEvent::Quit => Vec::new(),
        }
    }

    /// Move the value this page edits.
    fn nudge(&mut self, delta: i16) -> Vec<Command> {
        match self.screen {
            Screen::Levels => match self.level_focus {
                Level::Trim => {
                    let stepped = self.gain_db + delta as f32 * GAIN_STEP_DB;
                    self.gain_db = stepped.clamp(MIN_INPUT_TRIM_DB, MAX_INPUT_TRIM_DB);
                    // Continuous and audible: applying as it moves is what a
                    // knob does, so this is the exception to commit-on-select.
                    vec![Command::SetGain(self.gain_db)]
                }
                Level::Volume => {
                    // The pedal is the only source for this one — it is in no
                    // state message. Stepping from a value we invented could
                    // move the output level a long way in one press, so when it
                    // is unknown we ask instead of guessing.
                    let Some(current) = self.master_volume_db else {
                        return vec![Command::RequestMasterVolume];
                    };
                    let stepped = current + delta as f32 * VOLUME_STEP_DB;
                    let clamped = stepped.clamp(MIN_MASTER_VOLUME_DB, MAX_MASTER_VOLUME_DB);
                    self.master_volume_db = Some(clamped);
                    vec![Command::SetMasterVolume(clamped)]
                }
            },
            Screen::Slots | Screen::Stomp => {
                let span = MAX_PRESETS as i16;
                self.cursor = (((self.cursor as i16 + delta) % span + span) % span) as u8;
                Vec::new()
            }
        }
    }

    /// Apply what the cursor is on.
    fn commit(&mut self) -> Vec<Command> {
        if !self.connection.is_connected() {
            return Vec::new();
        }
        match self.screen {
            Screen::Levels => Vec::new(),
            Screen::Stomp => {
                // Stomp mode plays slot C, so that is what gets loaded.
                self.pending = Some(self.cursor);
                vec![Command::LoadSlot(Slot::C, self.cursor)]
            }
            Screen::Slots => {
                let target = self.selected;
                // Assigning the slot you are *hearing* changes the sound now;
                // assigning the other one is silent. Both are legitimate, so
                // the display says which is which rather than forbidding one.
                self.pending = Some(self.cursor);
                if self.active_slot == Some(target) {
                    vec![Command::AssignSlot(target, self.cursor)]
                } else {
                    // One write, not two: separate commands are each built from
                    // the same stale snapshot, so the second undoes the first.
                    vec![Command::LoadSlot(target, self.cursor)]
                }
            }
        }
    }

    /// Edit the slot on that side of the screen.
    ///
    /// Directional rather than a toggle: A is drawn on the left and B on the
    /// right, so pressing left should reach A whichever slot was being edited.
    /// A toggle made two presses of the same direction go there and back.
    fn select_slot(&mut self, slot: Slot) -> Vec<Command> {
        if self.screen != Screen::Slots {
            return Vec::new();
        }
        self.selected = slot;
        // Show what that slot already holds, so the cursor starts from the
        // current value rather than wherever it was left.
        if let Some(preset) = self.slot_preset(self.selected) {
            self.cursor = preset.min(MAX_PRESETS - 1);
        }
        Vec::new()
    }

    /// Move the Levels page to a row. Choosing is not changing, so nothing is
    /// sent to the pedal.
    fn focus_level(&mut self, level: Level) -> Vec<Command> {
        self.level_focus = level;
        Vec::new()
    }

    /// Ask for the state, then walk the presets one at a time.
    ///
    /// **Not twenty-one requests at once.** That burst is what left the pedal
    /// silent to everything including the handshake, needing a power cycle —
    /// and it was reachable from a button labelled "Refresh". Each preset is
    /// requested only once the previous one has answered, which is
    /// self-pacing: the pedal sets the rate, and a pedal that has stopped
    /// answering is never asked again.
    fn sync_all(&mut self) -> Vec<Command> {
        self.awaiting_preset = Some(0);
        // Master volume is asked for separately because it is in no state
        // message. One extra request, not twenty — it does not join the sweep.
        vec![
            Command::RequestState,
            Command::RequestMasterVolume,
            Command::RequestPreset(0),
        ]
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

    /// A state carrying a bypass flag, slot C, as stomp mode reports it.
    fn state_with_bypass(bypassed: bool) -> PedalEvent {
        let mut raw = vec![0u8; 64];
        let len = raw.len();
        raw[len - pinex_proto::state::offset_from_end::CURRENT_SLOT] = Slot::C as u8;
        raw[len - pinex_proto::state::offset_from_end::SLOT_C_PRESET] = 3;
        raw[len - pinex_proto::state::offset_from_end::BYPASS_MODE] = u8::from(bypassed);
        PedalEvent::StateChanged(PedalState::from_body(raw).unwrap())
    }

    /// A browser sitting on the Levels page.
    fn at_levels() -> PresetBrowser {
        let mut b = connected();
        b.apply(&state_with_active(0));
        while b.view().screen != Screen::Levels {
            b.handle(InputEvent::Page);
        }
        b
    }

    /// The pedal reporting its master volume, which is the only way it is known.
    fn master_volume_of(db: f32) -> PedalEvent {
        PedalEvent::ParamChanged(pinex_proto::message::ParamChange {
            index: pinex_proto::message::PARAM_MASTER_VOLUME,
            db,
            raw: pinex_proto::message::master_volume_to_wire(db),
        })
    }

    fn connected() -> PresetBrowser {
        let mut b = PresetBrowser::new();
        b.apply(&PedalEvent::Connected {
            firmware: "1.3.17".into(),
        });
        b
    }

    /// Connecting must not fire twenty-one requests at the pedal. That burst
    /// is what wedged it, and it was reachable from the Refresh button.
    #[test]
    fn connecting_asks_for_one_preset_at_a_time() {
        let mut b = PresetBrowser::new();
        let cmds = b.apply(&PedalEvent::Connected {
            firmware: "1.3.17".into(),
        });

        assert_eq!(
            cmds,
            vec![
                Command::RequestState,
                Command::RequestMasterVolume,
                Command::RequestPreset(0)
            ],
            "the sweep must start with one preset request, not twenty"
        );
        assert!(b.view().connection.is_connected());
    }

    /// ...but it must still fetch all of them, each answer pulling the next.
    #[test]
    fn every_preset_is_fetched_as_the_previous_one_answers() {
        let mut b = connected();
        let mut requested = vec![0u8];

        for index in 0..MAX_PRESETS {
            let cmds = b.apply(&named(index, &format!("PRESET {index}")));
            match cmds.as_slice() {
                [Command::RequestPreset(next)] => requested.push(*next),
                [] => assert_eq!(index, MAX_PRESETS - 1, "the sweep stopped early at {index}"),
                other => panic!("unexpected commands at {index}: {other:?}"),
            }
        }

        assert_eq!(
            requested.len(),
            MAX_PRESETS as usize,
            "not every preset asked for"
        );
        for index in 0..MAX_PRESETS {
            assert!(requested.contains(&index), "preset {index} never requested");
            assert!(b.name_at(index).is_some(), "preset {index} has no name");
        }
    }

    /// An unsolicited reply must not start a second sweep racing the first.
    #[test]
    fn an_unexpected_preset_reply_does_not_start_another_sweep() {
        let mut b = connected();
        // Mid-sweep, waiting on preset 0; a reply for 7 arrives unbidden.
        let cmds = b.apply(&named(7, "SURPRISE"));
        assert!(
            cmds.is_empty(),
            "an out-of-turn reply started another sweep: {cmds:?}"
        );
        assert_eq!(b.name_at(7), Some("SURPRISE"), "but its name is still kept");
    }

    /// Only the Slots page shows the A/B names, so only the Slots page has
    /// anything that scrolls.
    ///
    /// This reported "animating" from the A/B names whatever page was showing.
    /// On the Stomp page that made the app call the scroll path five times a
    /// second, and [`crate::panel::draw_scroll`] has no scrolling band there, so
    /// it falls back to a full redraw — which begins by clearing the screen.
    /// A full clear-and-repaint at 5 Hz is a visible flicker on the glass, and
    /// it costs about a third of a Pi 3 core to show a picture that never
    /// changed.
    #[test]
    fn only_the_page_that_shows_scrolling_names_animates() {
        let connection = Connection::Connected {
            firmware: "1.3.17".into(),
        };
        // Long enough that it cannot fit the slot column, so it genuinely wants
        // to scroll wherever scrolling applies.
        const LONG: &str = "MORNING GLORY - BRIGHT CUT 3";
        let view_on = |screen| View {
            screen,
            slot_names: [Some(LONG), Some(LONG), Some(LONG)],
            cursor_name: Some(LONG),
            ..View::stub(&connection)
        };

        assert!(
            view_on(Screen::Slots).animating(),
            "the Slots page shows the A/B names, so a long one must scroll"
        );
        for still in [Screen::Stomp, Screen::Levels] {
            assert!(
                !view_on(still).animating(),
                "{still:?} shows no A/B names, so nothing on it can scroll — \
                 claiming otherwise repaints the whole panel five times a second"
            );
        }
    }

    #[test]
    fn preset_names_land_at_their_index() {
        let mut b = connected();
        b.apply(&named(0, "TF BENSON PREAMP - 1"));
        b.apply(&named(15, "TF TILT - 1 ADV"));

        assert_eq!(b.view().cursor_name, Some("TF BENSON PREAMP - 1"));
        for _ in 0..15 {
            b.handle(InputEvent::Down);
        }
        assert_eq!(b.view().cursor_name, Some("TF TILT - 1 ADV"));
    }

    #[test]
    fn the_cursor_wraps_in_both_directions() {
        let mut b = connected();
        assert_eq!(b.view().cursor, 0);

        b.handle(InputEvent::Up);
        assert_eq!(b.view().cursor, MAX_PRESETS - 1, "0 must wrap back to 20");

        b.handle(InputEvent::Down);
        assert_eq!(b.view().cursor, 0, "20 must wrap forward to 1");
    }

    /// The rule the whole design rests on.
    #[test]
    fn selecting_a_preset_does_not_claim_it_is_active() {
        let mut b = connected();
        b.apply(&state_with_active(3));
        b.handle(InputEvent::Down);
        b.handle(InputEvent::Down);

        let cmds = b.handle(InputEvent::Select);

        // On the Slots page, Select targets the slot being edited. Which
        // command it is matters less than what it must NOT do.
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::AssignSlot(_, 5)] | [Command::LoadSlot(_, 5)]
            ),
            "should act on the cursor preset: {cmds:?}"
        );
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
        b.handle(InputEvent::Down);
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
        b.handle(InputEvent::Down);
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
        b.handle(InputEvent::Down);
        b.handle(InputEvent::Down);
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

    /// Left/right picks which level is being edited. The binding already
    /// exists — Mode does nothing on this page today — so no new control is
    /// needed for a second value.
    /// The axes match the picture: the rows are stacked, so up and down move
    /// between them, and each row's bar runs across, so left and right move it.
    #[test]
    fn up_and_down_choose_which_level_row_is_edited() {
        let mut b = at_levels();
        assert_eq!(
            b.view().level_focus,
            Level::Volume,
            "output level is what a player rides, so it is the default"
        );

        b.handle(InputEvent::Down);
        assert_eq!(b.view().level_focus, Level::Trim, "TRIM is the lower row");
        b.handle(InputEvent::Up);
        assert_eq!(b.view().level_focus, Level::Volume, "VOL is the upper row");

        // Directional, not a toggle: pressing the same way twice stays put.
        b.handle(InputEvent::Up);
        assert_eq!(b.view().level_focus, Level::Volume);
    }

    /// Left and right must not move the browsing cursor on this page — they
    /// belong to the bar now.
    #[test]
    fn the_level_rows_do_not_move_the_preset_cursor() {
        let mut b = at_levels();
        let before = b.view().cursor;
        b.handle(InputEvent::Left);
        b.handle(InputEvent::Right);
        assert_eq!(
            b.view().cursor,
            before,
            "the preset cursor is not on this page"
        );
    }

    /// Turning the output level must ask the pedal to change it, in dB.
    #[test]
    fn nudging_the_output_level_sets_master_volume() {
        let mut b = at_levels();
        b.apply(&master_volume_of(-10.0));

        let cmds = b.handle(InputEvent::Right);
        match cmds.as_slice() {
            [Command::SetMasterVolume(db)] => {
                assert!(*db > -10.0, "right must go louder, got {db}");
            }
            other => panic!("expected a master volume write, got {other:?}"),
        }
    }

    /// The range is the pedal's, and nothing outside it may be sent — this is
    /// the one control where a wrong number is expensive.
    #[test]
    fn the_output_level_stops_at_the_ends_of_the_pedals_range() {
        let mut b = at_levels();
        b.apply(&master_volume_of(MAX_MASTER_VOLUME_DB));
        for _ in 0..5 {
            b.handle(InputEvent::Right);
        }
        assert!(
            b.view().master_volume_db.unwrap() <= MAX_MASTER_VOLUME_DB,
            "went past the top"
        );

        b.apply(&master_volume_of(MIN_MASTER_VOLUME_DB));
        for _ in 0..5 {
            b.handle(InputEvent::Left);
        }
        assert!(
            b.view().master_volume_db.unwrap() >= MIN_MASTER_VOLUME_DB,
            "went past the bottom"
        );
    }

    /// With the focus on trim, this page behaves exactly as it did before.
    #[test]
    fn nudging_the_trim_still_sets_the_input_trim() {
        let mut b = at_levels();
        b.handle(InputEvent::Down); // focus the lower row, TRIM
        let cmds = b.handle(InputEvent::Right);
        assert!(
            matches!(cmds.as_slice(), [Command::SetGain(_)]),
            "expected an input trim write, got {cmds:?}"
        );
    }

    /// Master volume is in no state message, so before the pedal reports it we
    /// do not know it. Nudging from an invented starting point could jump the
    /// output level a long way; ask instead.
    #[test]
    fn nudging_the_output_level_before_the_pedal_reports_it_asks_rather_than_guesses() {
        let mut b = at_levels();
        assert_eq!(b.view().master_volume_db, None);

        let cmds = b.handle(InputEvent::Right);

        assert_eq!(
            cmds,
            vec![Command::RequestMasterVolume],
            "must ask, not invent a level"
        );
        assert_eq!(b.view().master_volume_db, None, "still unknown");
    }

    /// Stomping the footswitch must move the panel.
    ///
    /// Verified on hardware before this was written: toggling the switch in
    /// stomp mode changes exactly one byte of the state, end-relative 12, and
    /// the pedal announces it unsolicited. So the panel can follow the switch
    /// without polling — which matters, because polling is what wedges this
    /// pedal.
    #[test]
    fn the_bypass_state_follows_the_footswitch() {
        let mut b = connected();
        assert!(!b.view().bypassed, "not bypassed until the pedal says so");

        b.apply(&state_with_bypass(true));
        assert!(b.view().bypassed, "the pedal reported bypassed");

        b.apply(&state_with_bypass(false));
        assert!(!b.view().bypassed, "the pedal reported engaged again");
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

        b.handle(InputEvent::Up);
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
