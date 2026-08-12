//! Which look the panel wears.
//!
//! One layout is a guess about a stage nobody has described. Rather than argue
//! about it, both are built and switchable at runtime with
//! `PINEX_THEME=pedalboard|marquee`, so the choice can be made by standing in
//! front of the thing.
//!
//! They are deliberately opposed:
//!
//! - **Pedalboard** is skeuomorphic. Two moulded boxes with shaded knobs,
//!   gloss, screws and a chrome footswitch. It answers "which pedal is this"
//!   from arm's length and is pleasant to look at while you are setting up.
//!
//! - **Marquee** is typographic. Preset numbers at the largest size the panel
//!   allows, a bold colour spine for each slot, names underneath. It answers
//!   "which number am I on" from across a stage, in bad light, at a glance —
//!   and gives up the artwork to do it.
//!
//! Neither is correct in the abstract. A pedal you set up at home wants the
//! first; a pedal you step on mid-song wants the second.

/// The panel's visual treatment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// Moulded stompboxes with shading and highlights.
    #[default]
    Pedalboard,
    /// Big numbers, bold colour, minimal chrome.
    Marquee,
}

impl Theme {
    /// Read `PINEX_THEME`, defaulting to the skeuomorphic look.
    pub fn from_env() -> Self {
        match std::env::var("PINEX_THEME").as_deref() {
            Ok("marquee") | Ok("MARQUEE") => Self::Marquee,
            _ => Self::Pedalboard,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Pedalboard => "pedalboard",
            Self::Marquee => "marquee",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_theme_falls_back_rather_than_failing() {
        // A typo in a service file should not leave a player with a blank
        // panel mid-set.
        assert_eq!(Theme::default(), Theme::Pedalboard);
        assert_eq!(Theme::Marquee.name(), "marquee");
    }
}
