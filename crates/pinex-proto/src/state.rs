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

//! ## Why there are no offsets counted from the *start* of the body
//!
//! The fields near the start of the state live inside a list whose length
//! changes between firmware versions: 1.3.17 opens it with `b9 0e` — fourteen
//! elements — where the published 1.1.3 dump has `b9 0b`, eleven. A constant
//! offset into that region therefore reads a *different field* depending on
//! which firmware answered, silently, with no error to notice.
//!
//! Both reference implementations use such constants (`COLORS = 22` and
//! friends). Those values are correct for 1.3.17 and wrong for 1.1.3 — which
//! is the trap: they look right against whichever pedal you happen to own, and
//! break for someone else's.
//!
//! So anything in that region is located by *shape* instead — see
//! [`PedalState::preset_colors`], which finds the colour array by looking for
//! the `0xBA` list of exactly [`MAX_PRESETS`] triples, and reads both firmware
//! generations with the same code.
//!
//! The end-relative offsets below do not have this problem and are confirmed
//! against both generations, which is why the write path uses only those.

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

/// The input trim range the pedal accepts, from `protocol.md`'s annotation
/// (`-15.0` .. `15.0`).
pub const MIN_INPUT_TRIM_DB: f32 = -15.0;
pub const MAX_INPUT_TRIM_DB: f32 = 15.0;

/// The Tonex ONE stores 20 presets.
pub const MAX_PRESETS: u8 = 20;

/// Shortest body that can carry every field we address.
pub const MIN_STATE_LEN: usize = 23;

/// Preset slots. A/B are the pedal's two footswitch slots; C exists in stomp
/// mode.
///
/// Pinex presents a flat list of 20 presets and does not expose slots to the
/// player — see [`Slot::other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Slot {
    /// A is the default because A/B mode starts there.
    #[default]
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
    /// No per-preset colour array of the expected shape was present.
    NoColorArray,
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
            Self::NoColorArray => write!(
                f,
                "no {MAX_PRESETS}-entry colour array found in the state body"
            ),
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

    /// Whether the pedal is currently bypassed — the footswitch, not a setting.
    ///
    /// **Verified on hardware** (1.3.17, `probe_bypass`): stomping the switch in
    /// stomp mode changes exactly one byte, end-relative 12, toggling `0x01` and
    /// `0x00`, with every other byte identical. The pedal sends the new state
    /// unsolicited, so a display can follow the switch without polling — which
    /// matters, because sustained polling is what leaves this pedal silent.
    ///
    /// The name was transcribed from `protocol.md` long before any of that was
    /// checked, and "bypass mode" reads just as easily as a setting (true versus
    /// buffered bypass) as it does "bypassed right now". It is the latter, and
    /// the polarity is confirmed on the glass: `1` is bypassed.
    pub fn is_bypassed(&self) -> bool {
        self.bypass_mode() == 1
    }

    /// Index of the `0xBA` colour array, the anchor for the fields around it.
    ///
    /// The three literals immediately before the array are, in order, stomp
    /// mode, cab bypass and tuning mode. Anchoring to the array rather than to
    /// the start of the body is what makes these readable on both firmware
    /// generations: the array's *position* moves, but what sits beside it does
    /// not.
    fn color_array_index(&self) -> Result<usize, StateError> {
        let mut index = 0usize;
        while index < self.raw.len() {
            if self.raw[index] == 0xBA {
                let mut cursor = index;
                if read_color_array(&self.raw, &mut cursor).is_some() {
                    return Ok(index);
                }
            }
            index += 1;
        }
        Err(StateError::NoColorArray)
    }

    /// `0x00` = A/B mode, `0x01` = stomp mode.
    pub fn stomp_mode(&self) -> Result<u8, StateError> {
        let anchor = self.color_array_index()?;
        anchor
            .checked_sub(3)
            .map(|at| self.raw[at])
            .ok_or(StateError::NoColorArray)
    }

    /// `0x00` = off, `0x01` = on.
    pub fn cab_bypass(&self) -> Result<u8, StateError> {
        let anchor = self.color_array_index()?;
        anchor
            .checked_sub(2)
            .map(|at| self.raw[at])
            .ok_or(StateError::NoColorArray)
    }

    /// Switch between A/B and stomp mode. Returns the offset written.
    ///
    /// The pedal exposes slot C only in stomp mode, so this is what makes the
    /// third slot reachable at all.
    pub fn set_stomp_mode(&mut self, on: bool) -> Result<usize, StateError> {
        let anchor = self.color_array_index()?;
        let at = anchor.checked_sub(3).ok_or(StateError::NoColorArray)?;
        self.raw[at] = u8::from(on);
        Ok(at)
    }

    /// The per-preset RGB colours the pedal shows on its ring, one per preset.
    ///
    /// Found by *shape*, not by offset: the colour array is the `0xBA` list
    /// holding exactly [`MAX_PRESETS`] entries, each a 3-element list. That
    /// identification survives the firmware differences which make a constant
    /// offset here unsafe — the enclosing list has 11 elements on firmware 1.1.3
    /// and 14 on 1.3.17, so anything counted from the start of the body lands on
    /// a different field depending on who answered.
    ///
    /// Verified against captures from both firmware generations.
    pub fn preset_colors(&self) -> Result<Vec<[u8; 3]>, StateError> {
        let mut index = 0usize;
        while index < self.raw.len() {
            let Some(&tag) = self.raw.get(index) else {
                break;
            };

            if tag == 0xBA {
                let mut cursor = index;
                if let Some(colors) = read_color_array(&self.raw, &mut cursor) {
                    return Ok(colors);
                }
            }
            index += 1;
        }
        Err(StateError::NoColorArray)
    }

    /// Input trim in dB, the pedal's global gain.
    ///
    /// Lives immediately before the three mode flags, which sit before the
    /// colour array — so it is found by the same anchor rather than a constant.
    /// The value is the four bytes after an `0x88` f32 tag.
    fn input_trim_index(&self) -> Result<usize, StateError> {
        let anchor = self.color_array_index()?;
        // stomp/cab/tuning are anchor-3..anchor-1; the f32's four value bytes
        // end just before them, and the 0x88 tag precedes those.
        anchor.checked_sub(7).ok_or(StateError::NoColorArray)
    }

    /// The pedal's input trim, in dB.
    pub fn input_trim(&self) -> Result<f32, StateError> {
        let at = self.input_trim_index()?;
        let bytes: [u8; 4] = self.raw[at..at + 4]
            .try_into()
            .map_err(|_| StateError::NoColorArray)?;
        Ok(f32::from_le_bytes(bytes))
    }

    /// Set the input trim, clamped to the range the pedal accepts.
    ///
    /// Returns the four offsets written. Clamping rather than erroring, because
    /// a knob turned past its stop should stop, not fail.
    pub fn set_input_trim(&mut self, db: f32) -> Result<[usize; 4], StateError> {
        let at = self.input_trim_index()?;
        let clamped = db.clamp(MIN_INPUT_TRIM_DB, MAX_INPUT_TRIM_DB);
        self.raw[at..at + 4].copy_from_slice(&clamped.to_le_bytes());
        Ok([at, at + 1, at + 2, at + 3])
    }

    /// A4 tuning reference in Hz (e.g. 440).
    pub fn tuning_reference_hz(&self) -> u16 {
        let at = self.index_from_end(offset_from_end::TUNING_REF);
        u16::from_le_bytes([self.raw[at], self.raw[at + 1]])
    }

    /// Tempo in beats per minute.
    pub fn tempo_bpm(&self) -> f32 {
        let at = self.index_from_end(offset_from_end::BPM);
        f32::from_le_bytes([
            self.raw[at],
            self.raw[at + 1],
            self.raw[at + 2],
            self.raw[at + 3],
        ])
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

    /// Put `preset` into `slot` without changing which slot is playing.
    ///
    /// This is what "choose which one is A and which is B" needs: assigning the
    /// slot you are *not* hearing must not change the sound. Assigning the slot
    /// you *are* hearing changes it immediately, which is the caller's business
    /// to warn about, not ours to prevent.
    ///
    /// Returns the offsets written.
    pub fn assign_slot(&mut self, slot: Slot, preset: u8) -> Result<Vec<usize>, StateError> {
        let mut touched = vec![
            self.set_slot_preset(slot, preset)?,
            self.force_direct_monitoring(),
        ];
        touched.sort_unstable();
        touched.dedup();
        Ok(touched)
    }

    /// Put `preset` into `slot` and switch to it, in one edit.
    ///
    /// One operation rather than two on purpose. Each write is built from the
    /// pedal's last reported state, so issuing "assign" and "switch" separately
    /// means the second is built from a snapshot taken *before* the first
    /// applied — and silently undoes it. Composing them keeps a single write
    /// with a single diff assertion.
    pub fn load_slot(&mut self, slot: Slot, preset: u8) -> Result<Vec<usize>, StateError> {
        let mut touched = vec![
            self.set_slot_preset(slot, preset)?,
            self.set_active_slot(slot),
            self.force_direct_monitoring(),
        ];
        touched.sort_unstable();
        touched.dedup();
        Ok(touched)
    }

    /// Switch to `slot` without changing what any slot holds.
    ///
    /// The A/B stomp: both sounds are already loaded, so this is the one that
    /// has to be instant and silent.
    pub fn switch_to_slot(&mut self, slot: Slot) -> Result<Vec<usize>, StateError> {
        let mut touched = vec![self.set_active_slot(slot), self.force_direct_monitoring()];
        touched.sort_unstable();
        touched.dedup();
        Ok(touched)
    }

    /// Change the playing preset, by whichever route this pedal actually honours.
    ///
    /// **Stomp mode (active slot C) needs the opposite of the documented
    /// approach, and hardware is how we found out.** Staging into another slot
    /// and switching to it — what both reference implementations do — is
    /// accepted by the pedal and then silently reverted about a second later:
    ///
    /// ```text
    /// StateChanged: preset 0, slot A   <- our write landed
    /// StateChanged: preset 1, slot C   <- the pedal put it back
    /// ```
    ///
    /// Writing the preset into the *current* slot in place does stick, and costs
    /// a single byte. So:
    ///
    /// | Active slot | Route | Verified on firmware 1.3.17 |
    /// |---|---|---|
    /// | C (stomp) | write in place | yes — a slot switch is reverted here |
    /// | A or B | stage into the other slot, then switch | yes |
    ///
    /// Both routes are now confirmed against hardware. In A/B mode all three
    /// candidate strategies stick, and the stage-and-switch one was kept because
    /// it is the only one that preserves the double buffering: successive
    /// changes alternate `[B, A, B, A]`, and the slot being heard still holds
    /// what it held. That is what keeps a preset change inaudible, and it is
    /// measured rather than assumed — see
    /// `crates/pinex/examples/probe_ab_alternation.rs`.
    ///
    /// Returns the offsets written, sorted, for the [`diff_offsets`] assertion.
    pub fn change_preset(&mut self, preset: u8) -> Result<Vec<usize>, StateError> {
        match self.active_slot()? {
            Slot::C => {
                let mut touched = vec![
                    self.set_slot_preset(Slot::C, preset)?,
                    self.force_direct_monitoring(),
                ];
                touched.sort_unstable();
                touched.dedup();
                Ok(touched)
            }
            Slot::A | Slot::B => self.stage_preset_in_inactive_slot(preset),
        }
    }

    /// Stage `preset` into the inactive slot and switch to it — the glitch-free
    /// preset change described on [`Slot::other`].
    ///
    /// Only correct in A/B mode. See [`Self::change_preset`], which picks the
    /// route the pedal actually honours.
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

/// Read a `0xBA` list of exactly [`MAX_PRESETS`] RGB triples at `*index`.
///
/// Returns `None` — rather than erroring — if the shape does not match, because
/// the caller is scanning candidate positions and a mismatch just means "not
/// this one". Every read is bounds-checked: this walks bytes from a device we
/// do not control.
fn read_color_array(buf: &[u8], index: &mut usize) -> Option<Vec<[u8; 3]>> {
    let mut cursor = *index;
    if *buf.get(cursor)? != 0xBA {
        return None;
    }
    cursor += 1;
    if *buf.get(cursor)? != MAX_PRESETS {
        return None;
    }
    cursor += 1;

    let mut colors = Vec::with_capacity(MAX_PRESETS as usize);
    for _ in 0..MAX_PRESETS {
        if *buf.get(cursor)? != 0xB9 || *buf.get(cursor + 1)? != 3 {
            return None;
        }
        cursor += 2;
        let mut channel = [0u8; 3];
        for slot in channel.iter_mut() {
            // Channels above 0x7F carry an 0x80 tag; below, the byte is itself.
            *slot = match *buf.get(cursor)? {
                0x80 => {
                    cursor += 2;
                    *buf.get(cursor - 1)?
                }
                literal => {
                    cursor += 1;
                    literal
                }
            };
        }
        colors.push(channel);
    }

    *index = cursor;
    Some(colors)
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
