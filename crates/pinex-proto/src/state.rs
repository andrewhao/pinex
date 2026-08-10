//! Pedal state, held as raw bytes and edited in place.
//!
//! **The pedal has no "set preset N" command.** Changing anything means sending
//! the whole state back. The obvious implementation — decode to a struct, mutate,
//! re-encode — risks writing back a field we misparsed and never meant to touch,
//! and the state contains fields nobody has reverse-engineered.
//!
//! So we never re-encode. [`PedalState`] keeps the raw body verbatim and patches
//! individual bytes at known offsets; unknown fields survive *by construction*
//! because they are never decoded. Both reference implementations do this, and it
//! turns a correctness argument into a structural guarantee.
//!
//! [`diff_offsets`] exists so a caller can assert that a write touched exactly
//! the offsets it intended, and nothing else.

/// Offsets counted from the start of the state body.
///
/// Corrected against the captured message in
/// `tests/fixtures/bodies/state_changed.body.bin`; the previous values were all
/// five bytes too high and addressed the wrong fields. `tests/state_offsets.rs`
/// pins each one to the annotation its capture carries.
///
/// Unlike [`offset_from_end`], these are read-only: nothing in the write path
/// patches through them, so the earlier error could not have corrupted a pedal.
pub mod offset_from_start {
    /// f32: -15.0 .. +15.0. Points at the value bytes, past the `0x88` tag.
    pub const INPUT_TRIM: usize = 5;
    /// `0x00` = A/B mode, `0x01` = stomp mode.
    ///
    /// **Inferred, not annotated.** The capture leaves this byte unlabelled;
    /// it is identified only by sitting immediately before `cabsimBypass`, so
    /// it is the least trustworthy constant here.
    pub const STOMP_MODE: usize = 14;
    /// `0x00` = off, `0x01` = on
    pub const CAB_BYPASS: usize = 15;
    /// `0x00` = mute, `0x01` = through
    pub const TUNING_MODE: usize = 16;
    /// The `0xBA` header of the per-preset RGB colour array.
    pub const COLORS: usize = 17;
}

/// Offsets counted back from the end of the state body, as `len - N`.
///
/// Confirmed independently by `vit3k/tonex_controller` (`parseState`, `setSlot`,
/// `changePreset`) and `Builty/TonexOneController` (`TONEX_STATE_OFFSET_END_*`).
pub mod offset_from_end {
    pub const BPM: usize = 4;
    /// `0x00` = global, `0x01` = preset
    pub const TEMPO_SOURCE: usize = 6;
    /// `0x00` = off, `0x01` = on. See [`super::PedalState::force_direct_monitoring`].
    pub const DIRECT_MONITOR: usize = 7;
    pub const TUNING_REF: usize = 9;
    pub const CURRENT_SLOT: usize = 11;
    /// `0x00` = off, `0x01` = on
    pub const BYPASS_MODE: usize = 12;
    pub const SLOT_C_PRESET: usize = 14;
    pub const SLOT_B_PRESET: usize = 16;
    pub const SLOT_A_PRESET: usize = 18;
}

/// The Tonex ONE stores 20 presets.
pub const MAX_PRESETS: u8 = 20;

/// Shortest body that can carry every field we address.
pub const MIN_STATE_LEN: usize = 23;

/// Preset slots. A/B are the pedal's two footswitch slots; C exists in stomp
/// mode.
///
/// Pinex presents a flat list of 20 presets and does not expose slots to the
/// player — see [`Slot::other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Slot {
    A = 0,
    B = 1,
    C = 2,
}

impl Slot {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::A),
            1 => Some(Self::B),
            2 => Some(Self::C),
            _ => None,
        }
    }

    /// The slot to stage the next preset into.
    ///
    /// Loading a preset into the slot currently being heard produces an audible
    /// artifact. Writing to the *other* slot and then switching to it does not,
    /// so A/B act as a double buffer. This is `switchSilently` in
    /// `vit3k/tonex_controller`. Stomp-mode slot C maps back to A.
    pub fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B | Self::C => Self::A,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    /// Body is too short to address the documented fields.
    TooShort { len: usize, min: usize },
    /// Preset index outside `0..MAX_PRESETS`.
    PresetOutOfRange { preset: u8, max: u8 },
    /// Byte at the active-slot offset was not a slot we recognise.
    UnknownSlot { value: u8 },
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort { len, min } => {
                write!(
                    f,
                    "state body of {len} bytes is shorter than the {min}-byte minimum"
                )
            }
            Self::PresetOutOfRange { preset, max } => {
                write!(f, "preset {preset} out of range (0..{max})")
            }
            Self::UnknownSlot { value } => write!(f, "unknown slot byte {value:#04x}"),
        }
    }
}

impl std::error::Error for StateError {}

/// Pedal state. The raw bytes are authoritative; accessors are a read-only view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PedalState {
    raw: Vec<u8>,
}

impl PedalState {
    /// Wrap a state body, rejecting anything too short to address.
    pub fn from_body(raw: Vec<u8>) -> Result<Self, StateError> {
        if raw.len() < MIN_STATE_LEN {
            return Err(StateError::TooShort {
                len: raw.len(),
                min: MIN_STATE_LEN,
            });
        }
        Ok(Self { raw })
    }

    /// The verbatim body, ready to be echoed back to the pedal.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    pub fn len(&self) -> usize {
        self.raw.len()
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// Resolve an end-relative offset to an absolute index.
    fn index_from_end(&self, offset: usize) -> usize {
        self.raw.len() - offset
    }

    fn slot_offset(slot: Slot) -> usize {
        match slot {
            Slot::A => offset_from_end::SLOT_A_PRESET,
            Slot::B => offset_from_end::SLOT_B_PRESET,
            Slot::C => offset_from_end::SLOT_C_PRESET,
        }
    }

    /// Preset index currently loaded in `slot`.
    pub fn slot_preset(&self, slot: Slot) -> u8 {
        self.raw[self.index_from_end(Self::slot_offset(slot))]
    }

    /// Which slot the pedal is playing.
    pub fn active_slot(&self) -> Result<Slot, StateError> {
        let value = self.raw[self.index_from_end(offset_from_end::CURRENT_SLOT)];
        Slot::from_u8(value).ok_or(StateError::UnknownSlot { value })
    }

    /// Preset the player is actually hearing.
    pub fn active_preset(&self) -> Result<u8, StateError> {
        Ok(self.slot_preset(self.active_slot()?))
    }

    pub fn direct_monitoring(&self) -> u8 {
        self.raw[self.index_from_end(offset_from_end::DIRECT_MONITOR)]
    }

    pub fn bypass_mode(&self) -> u8 {
        self.raw[self.index_from_end(offset_from_end::BYPASS_MODE)]
    }

    pub fn stomp_mode(&self) -> u8 {
        self.raw[offset_from_start::STOMP_MODE]
    }

    pub fn cab_bypass(&self) -> u8 {
        self.raw[offset_from_start::CAB_BYPASS]
    }

    /// Load `preset` into `slot`. Returns the offset written.
    pub fn set_slot_preset(&mut self, slot: Slot, preset: u8) -> Result<usize, StateError> {
        if preset >= MAX_PRESETS {
            return Err(StateError::PresetOutOfRange {
                preset,
                max: MAX_PRESETS,
            });
        }
        let at = self.index_from_end(Self::slot_offset(slot));
        self.raw[at] = preset;
        Ok(at)
    }

    /// Switch the pedal to `slot`. Returns the offset written.
    pub fn set_active_slot(&mut self, slot: Slot) -> usize {
        let at = self.index_from_end(offset_from_end::CURRENT_SLOT);
        self.raw[at] = slot as u8;
        at
    }

    /// Force direct monitoring on. Returns the offset written.
    ///
    /// **Not optional.** Being connected over USB can mute the pedal's output;
    /// `Builty/TonexOneController` sets this byte on every state write with the
    /// comment *"make sure direct monitoring is on so sound not muted from USB
    /// connection"*. A write that omits it can silence the pedal, which reads as
    /// broken hardware rather than a software bug.
    pub fn force_direct_monitoring(&mut self) -> usize {
        let at = self.index_from_end(offset_from_end::DIRECT_MONITOR);
        self.raw[at] = 1;
        at
    }

    /// Stage `preset` into the inactive slot and switch to it — the glitch-free
    /// preset change described on [`Slot::other`].
    ///
    /// Returns the offsets written, sorted, for the [`diff_offsets`] assertion.
    pub fn stage_preset_in_inactive_slot(&mut self, preset: u8) -> Result<Vec<usize>, StateError> {
        let target = self.active_slot()?.other();
        let mut touched = vec![
            self.set_slot_preset(target, preset)?,
            self.set_active_slot(target),
            self.force_direct_monitoring(),
        ];
        touched.sort_unstable();
        touched.dedup();
        Ok(touched)
    }
}

/// Offsets at which two buffers differ.
///
/// Differing lengths are reported as a difference at every trailing offset, so a
/// length change can never be mistaken for "no change".
pub fn diff_offsets(before: &[u8], after: &[u8]) -> Vec<usize> {
    let common = before.len().min(after.len());
    let mut out: Vec<usize> = (0..common).filter(|&i| before[i] != after[i]).collect();
    out.extend(common..before.len().max(after.len()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body long enough to address every field, with recognisable filler so an
    /// unintended write shows up.
    fn sample_body() -> Vec<u8> {
        let mut raw: Vec<u8> = (0..64u8)
            .map(|i| i.wrapping_mul(3).wrapping_add(7))
            .collect();
        let len = raw.len();
        raw[len - offset_from_end::SLOT_A_PRESET] = 4;
        raw[len - offset_from_end::SLOT_B_PRESET] = 9;
        raw[len - offset_from_end::SLOT_C_PRESET] = 2;
        raw[len - offset_from_end::CURRENT_SLOT] = Slot::A as u8;
        raw[len - offset_from_end::DIRECT_MONITOR] = 0;
        raw
    }

    #[test]
    fn reads_slots_and_active_preset() {
        let state = PedalState::from_body(sample_body()).unwrap();
        assert_eq!(state.slot_preset(Slot::A), 4);
        assert_eq!(state.slot_preset(Slot::B), 9);
        assert_eq!(state.slot_preset(Slot::C), 2);
        assert_eq!(state.active_slot().unwrap(), Slot::A);
        assert_eq!(state.active_preset().unwrap(), 4);
    }

    #[test]
    fn short_body_errors_rather_than_panicking() {
        for len in 0..MIN_STATE_LEN {
            assert!(matches!(
                PedalState::from_body(vec![0; len]),
                Err(StateError::TooShort { .. })
            ));
        }
        assert!(PedalState::from_body(vec![0; MIN_STATE_LEN]).is_ok());
    }

    #[test]
    fn unknown_slot_byte_is_reported() {
        let mut raw = sample_body();
        let len = raw.len();
        raw[len - offset_from_end::CURRENT_SLOT] = 0xFF;
        let state = PedalState::from_body(raw).unwrap();
        assert!(matches!(
            state.active_slot(),
            Err(StateError::UnknownSlot { value: 0xFF })
        ));
    }

    #[test]
    fn out_of_range_preset_is_rejected_and_leaves_state_untouched() {
        let mut state = PedalState::from_body(sample_body()).unwrap();
        let before = state.raw().to_vec();

        assert!(matches!(
            state.set_slot_preset(Slot::A, MAX_PRESETS),
            Err(StateError::PresetOutOfRange { .. })
        ));
        assert!(matches!(
            state.set_slot_preset(Slot::A, 255),
            Err(StateError::PresetOutOfRange { .. })
        ));
        assert_eq!(state.raw(), &before[..], "a rejected write must not mutate");
    }

    #[test]
    fn setting_a_preset_touches_exactly_one_byte() {
        let mut state = PedalState::from_body(sample_body()).unwrap();
        let before = state.raw().to_vec();

        let at = state.set_slot_preset(Slot::B, 17).unwrap();

        assert_eq!(diff_offsets(&before, state.raw()), vec![at]);
        assert_eq!(state.slot_preset(Slot::B), 17);
    }

    #[test]
    fn staging_a_preset_touches_only_the_allowlisted_offsets() {
        let mut state = PedalState::from_body(sample_body()).unwrap();
        let before = state.raw().to_vec();
        let len = before.len();

        let touched = state.stage_preset_in_inactive_slot(11).unwrap();

        // Active slot was A, so the staged preset lands in B.
        let expected = {
            let mut e = vec![
                len - offset_from_end::SLOT_B_PRESET,
                len - offset_from_end::CURRENT_SLOT,
                len - offset_from_end::DIRECT_MONITOR,
            ];
            e.sort_unstable();
            e
        };

        assert_eq!(touched, expected);
        assert_eq!(
            diff_offsets(&before, state.raw()),
            expected,
            "no byte outside the allowlist may change"
        );

        assert_eq!(state.active_slot().unwrap(), Slot::B);
        assert_eq!(state.active_preset().unwrap(), 11);
        assert_eq!(state.direct_monitoring(), 1);
        assert_eq!(
            state.slot_preset(Slot::A),
            4,
            "the slot being heard is untouched"
        );
    }

    #[test]
    fn staging_alternates_slots_so_the_audible_slot_is_never_overwritten() {
        let mut state = PedalState::from_body(sample_body()).unwrap();
        let mut heard = Vec::new();

        for preset in [1u8, 2, 3, 4] {
            let previous = state.active_preset().unwrap();
            let target = state.active_slot().unwrap().other();
            assert_ne!(
                target,
                state.active_slot().unwrap(),
                "must stage into a slot that is not playing"
            );

            state.stage_preset_in_inactive_slot(preset).unwrap();

            // The previously-heard preset is still intact in the other slot.
            assert_eq!(
                state.slot_preset(state.active_slot().unwrap().other()),
                previous
            );
            heard.push(state.active_preset().unwrap());
        }

        assert_eq!(heard, vec![1, 2, 3, 4]);
    }

    #[test]
    fn length_is_preserved_by_every_mutation() {
        let mut state = PedalState::from_body(sample_body()).unwrap();
        let len = state.len();
        state.set_slot_preset(Slot::A, 3).unwrap();
        state.set_active_slot(Slot::C);
        state.force_direct_monitoring();
        state.stage_preset_in_inactive_slot(8).unwrap();
        assert_eq!(state.len(), len, "state writes must never resize the body");
    }

    #[test]
    fn diff_offsets_reports_length_changes() {
        assert_eq!(diff_offsets(&[1, 2, 3], &[1, 2, 3]), Vec::<usize>::new());
        assert_eq!(diff_offsets(&[1, 2, 3], &[1, 9, 3]), vec![1]);
        assert_eq!(diff_offsets(&[1, 2, 3], &[1, 2]), vec![2]);
        assert_eq!(diff_offsets(&[1, 2], &[1, 2, 3, 4]), vec![2, 3]);
    }

    #[test]
    fn slot_c_stages_back_into_a() {
        assert_eq!(Slot::A.other(), Slot::B);
        assert_eq!(Slot::B.other(), Slot::A);
        assert_eq!(Slot::C.other(), Slot::A);
    }
}
