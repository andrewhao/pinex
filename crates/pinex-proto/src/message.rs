//! Message builders and header parsing.
//!
//! Every message is `0xB9 0x03`, a tagged type, a tagged size, a tagged unknown
//! field, then the body — all wrapped by [`crate::frame`].

use crate::frame::encode_frame;
use crate::state::{PedalState, MAX_PRESETS};
use crate::value::{read_int, read_list_header, skip_value, ValueError};

/// Message type of a state update, sent by the pedal and echoed back on write.
pub const TYPE_STATE_UPDATE: u16 = 0x0306;
/// Message type of the Hello response.
pub const TYPE_HELLO: u16 = 0x02;
/// Message type used to *request* preset details.
pub const TYPE_PRESET_REQUEST: u16 = 0x0300;
/// Message type of the pedal's acknowledgement of a state write.
///
/// A five-byte message with an empty body (`b9 03 05 00 0b`), sent after a state
/// write. Undocumented anywhere we have seen; observed on firmware 1.3.17 and
/// identified only by when it arrives. It is recognised so it stops being
/// reported as a parse failure, but nothing is inferred from it — notably it is
/// **not** evidence the write stuck, since the pedal sends it even when it is
/// about to revert the change.
pub const TYPE_WRITE_ACK: u16 = 0x0005;
/// Message type of a preset-details *response*.
///
/// Confirmed against our own pedal (firmware 1.3.17): every reply to
/// `request_preset` carries `0x0304`. See
/// `tests/hardware_captures.rs::preset_responses_carry_their_index_and_name`.
pub const TYPE_PRESET_RESPONSE: u16 = 0x0304;

/// Message type carrying a single parameter value, in both directions.
///
/// We send it to set master volume; the pedal sends it back reporting the value
/// it settled on. `Builty/TonexOneController` calls the inbound one
/// `TYPE_PARAM_CHANGED`. Only firmware new enough for Editor support has it —
/// ours (1.3.17) qualifies.
pub const TYPE_PARAM_CHANGED: u16 = 0x0309;

/// Message type that asks the pedal for its master volume.
pub const TYPE_MASTER_VOLUME_REQUEST: u16 = 0x030D;

/// Parameter index of master volume within [`TYPE_PARAM_CHANGED`].
pub const PARAM_MASTER_VOLUME: u16 = 0x0000;

/// The master-volume range, in decibels.
///
/// From `Builty/TonexOneController`, which maps the ToneX One's control onto the
/// range the larger ToneX exposes. The pedal itself clamps nothing, and neither
/// does the reference — see [`set_master_volume`].
pub const MIN_MASTER_VOLUME_DB: f32 = -40.0;
pub const MAX_MASTER_VOLUME_DB: f32 = 3.0;

/// The width of the decibel range, which is also the scale factor to the wire.
const MASTER_VOLUME_DB_SPAN: f32 = MAX_MASTER_VOLUME_DB - MIN_MASTER_VOLUME_DB;

/// The pedal's own master-volume scale runs 0..10, not in decibels.
const MASTER_VOLUME_WIRE_MAX: f32 = 10.0;

/// Decibels to the 0..10 scale the pedal speaks.
///
/// **The pedal does not take decibels.** The reference converts on the way in
/// (`((db + 40) / 43) * 10`) and back on the way out, and only the converted
/// value ever reaches the wire. Sending decibels straight through would put
/// `-40` into a control that reads `0..10` — which is not a quiet pedal, it is
/// an out-of-range one.
pub fn master_volume_to_wire(db: f32) -> f32 {
    // NaN collapses to the floor rather than propagating onto the wire: clamp
    // alone would pass it straight through.
    if db.is_nan() {
        return 0.0;
    }
    let clamped = db.clamp(MIN_MASTER_VOLUME_DB, MAX_MASTER_VOLUME_DB);
    ((clamped - MIN_MASTER_VOLUME_DB) / MASTER_VOLUME_DB_SPAN) * MASTER_VOLUME_WIRE_MAX
}

/// The 0..10 scale back to decibels, for reading the pedal's own answer.
pub fn master_volume_from_wire(raw: f32) -> f32 {
    if raw.is_nan() {
        return MIN_MASTER_VOLUME_DB;
    }
    let clamped = raw.clamp(0.0, MASTER_VOLUME_WIRE_MAX);
    (clamped / MASTER_VOLUME_WIRE_MAX) * MASTER_VOLUME_DB_SPAN + MIN_MASTER_VOLUME_DB
}

/// A single parameter value the pedal has reported.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamChange {
    /// Which parameter. [`PARAM_MASTER_VOLUME`] is the only one we act on.
    pub index: u16,
    /// The value in decibels, already off the pedal's 0..10 scale.
    ///
    /// Only meaningful for master volume; the other 111 parameters have their
    /// own units and are not interpreted here.
    pub db: f32,
    /// The raw value exactly as the pedal sent it, before any conversion.
    pub raw: f32,
}

impl ParamChange {
    pub fn is_master_volume(&self) -> bool {
        self.index == PARAM_MASTER_VOLUME
    }
}

/// Header marker every message opens with.
const HEADER_MARKER: [u8; 2] = [0xB9, 0x03];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    Hello,
    StateUpdate,
    PresetResponse,
    /// Acknowledgement of a state write. Carries no payload and no promise.
    WriteAck,
    /// A single parameter's value, in either direction.
    ParamChanged,
    Unknown(u16),
}

impl MessageType {
    pub fn from_code(code: u16) -> Self {
        match code {
            TYPE_HELLO => Self::Hello,
            TYPE_STATE_UPDATE => Self::StateUpdate,
            TYPE_PRESET_RESPONSE => Self::PresetResponse,
            TYPE_WRITE_ACK => Self::WriteAck,
            TYPE_PARAM_CHANGED => Self::ParamChanged,
            other => Self::Unknown(other),
        }
    }
}

/// How much preset data to ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetDetail {
    /// ~2 KB summary. Enough for the preset name; what Pinex uses.
    Summary = 0x00,
    /// ~30 KB full parameter dump.
    Full = 0x01,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub msg_type: MessageType,
    /// Declared body size.
    pub size: u16,
    /// Purpose unknown; preserved so it can be surfaced when parsing breaks.
    pub unknown: u16,
    /// Offset at which the body begins.
    pub body_offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageError {
    /// Missing the `0xB9 0x03` marker.
    BadMarker {
        found: Vec<u8>,
    },
    /// Declared size disagrees with the bytes actually present.
    SizeMismatch {
        declared: u16,
        actual: usize,
    },
    /// Preset index outside `0..MAX_PRESETS`.
    PresetOutOfRange {
        preset: u8,
        max: u8,
    },
    /// Structure did not match what the captured fixtures show.
    UnexpectedShape {
        what: &'static str,
    },
    /// A write would have modified a byte outside the intended set. Never sent.
    UnsafeWrite {
        offset: usize,
        intended: Vec<usize>,
    },
    Value(ValueError),
}

impl From<ValueError> for MessageError {
    fn from(err: ValueError) -> Self {
        Self::Value(err)
    }
}

impl std::fmt::Display for MessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMarker { found } => {
                write!(f, "expected header marker b9 03, found {found:02x?}")
            }
            Self::SizeMismatch { declared, actual } => {
                write!(
                    f,
                    "header declares {declared} body bytes but {actual} are present"
                )
            }
            Self::PresetOutOfRange { preset, max } => {
                write!(f, "preset {preset} out of range (0..{max})")
            }
            Self::UnexpectedShape { what } => write!(f, "unexpected message shape: {what}"),
            Self::UnsafeWrite { offset, intended } => write!(
                f,
                "refusing to transmit: write would change offset {offset}, \
                 which is not among the intended offsets {intended:?}"
            ),
            Self::Value(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for MessageError {}

/// Handshake. The pedal replies with its firmware version.
pub fn hello() -> Vec<u8> {
    encode_frame(&[
        0xb9, 0x03, 0x00, 0x82, 0x04, 0x00, 0x80, 0x0b, 0x01, 0xb9, 0x02, 0x02, 0x0b,
    ])
}

/// Ask for the complete pedal state.
pub fn request_state() -> Vec<u8> {
    encode_frame(&[
        0xb9, 0x03, 0x00, 0x82, 0x06, 0x00, 0x80, 0x0b, 0x03, 0xb9, 0x02, 0x81, 0x06, 0x03, 0x0b,
    ])
}

/// Ask for one preset's data. `preset` is 0-based.
///
/// Preset names are fetched by walking `0..MAX_PRESETS`, issuing the next request
/// only once the previous response arrives.
pub fn request_preset(preset: u8, detail: PresetDetail) -> Result<Vec<u8>, MessageError> {
    if preset >= MAX_PRESETS {
        return Err(MessageError::PresetOutOfRange {
            preset,
            max: MAX_PRESETS,
        });
    }
    let mut payload = vec![
        0xb9, 0x03, 0x81, 0x00, 0x03, 0x82, 0x06, 0x00, 0x80, 0x0b, 0x03, 0xb9, 0x04, 0x0b, 0x01,
        0x00, 0x00,
    ];
    payload[15] = preset;
    payload[16] = detail as u8;
    Ok(encode_frame(&payload))
}

/// Set the pedal's master volume, in decibels.
///
/// Unlike every other write in this crate, this does **not** go through the
/// state: master volume is not in the state message at all. There is nothing to
/// patch and nothing to diff-assert, so the safety this crate normally gets
/// structurally has to be spelled out — hence the clamp in
/// [`master_volume_to_wire`], which the reference implementation does not have.
///
/// Loudness is the one setting where a wrong number costs something, so pair
/// this with [`request_master_volume`] and believe the pedal's answer rather
/// than assuming the write landed.
pub fn set_master_volume(db: f32) -> Vec<u8> {
    let mut payload = vec![
        0xb9, 0x03, 0x81, 0x09, 0x03, 0x82, 0x0a, 0x00, 0x80, 0x0b, 0x03, // message
        0xb9, 0x04, 0x03, 0x00, 0x00, 0x88, // payload up to the float marker
    ];
    payload.extend_from_slice(&master_volume_to_wire(db).to_le_bytes());
    encode_frame(&payload)
}

/// Ask the pedal what its master volume is.
///
/// The reply is a [`TYPE_PARAM_CHANGED`] message; feed it to
/// [`parse_param_changed`]. This is the only way to know the value — it appears
/// nowhere in the state.
pub fn request_master_volume() -> Vec<u8> {
    encode_frame(&[
        0xb9, 0x03, 0x81, 0x0d, 0x03, 0x82, 0x05, 0x00, 0x80, 0x0b, 0x03, 0xb9, 0x03, 0x03, 0x00,
        0x00,
    ])
}

/// Read a single-parameter report from the pedal.
///
/// Located by scanning for the `B9 04 03` marker rather than by a fixed offset,
/// matching the reference's `memmem` and for the same reason every other lookup
/// in this crate is structural: fixed offsets into this protocol have already
/// been wrong once between firmware generations.
pub fn parse_param_changed(body: &[u8]) -> Result<ParamChange, MessageError> {
    const MARKER: [u8; 3] = [0xB9, 0x04, 0x03];

    let start = body
        .windows(MARKER.len())
        .position(|window| window == MARKER)
        .ok_or(MessageError::UnexpectedShape {
            what: "no B9 04 03 parameter marker in a param-changed message",
        })?;

    // marker, then a 2-byte little-endian index, then the 0x88 f32 tag.
    let index_at = start + MARKER.len();
    let value_at = index_at + 3;
    if body.len() < value_at + 4 {
        return Err(MessageError::UnexpectedShape {
            what: "param-changed message truncated before its value",
        });
    }
    if body[value_at - 1] != 0x88 {
        return Err(MessageError::UnexpectedShape {
            what: "param-changed value is not tagged as an f32",
        });
    }

    let index = u16::from_le_bytes([body[index_at], body[index_at + 1]]);
    let raw = f32::from_le_bytes([
        body[value_at],
        body[value_at + 1],
        body[value_at + 2],
        body[value_at + 3],
    ]);
    Ok(ParamChange {
        index,
        db: master_volume_from_wire(raw),
        raw,
    })
}

/// Write state back to the pedal.
///
/// The body is [`PedalState`]'s raw bytes, unmodified except for the individual
/// offsets patched in place — nothing is re-encoded. See [`crate::state`].
pub fn write_state(state: &PedalState) -> Vec<u8> {
    let raw = state.raw();
    let size = (raw.len() & 0xFFFF) as u16;
    let mut payload = vec![
        0xb9,
        0x03,
        0x81,
        0x06,
        0x03,
        0x82,
        (size & 0xFF) as u8,
        (size >> 8) as u8,
        0x80,
        0x0b,
        0x03,
    ];
    payload.extend_from_slice(raw);
    encode_frame(&payload)
}

/// Index of the firmware-version element within the Hello response body list.
///
/// Derived from the captured response in `tests/fixtures/hello_response.bin`,
/// whose own annotation identifies element 3 as the version.
const HELLO_FIRMWARE_ELEMENT: u16 = 3;

/// Extract the firmware version string from a Hello response body.
///
/// Returns e.g. `"1.1.3"`. Errors rather than guessing if the shape differs —
/// a firmware update changing this layout must be loud, not silent.
pub fn parse_hello(body: &[u8]) -> Result<String, MessageError> {
    let header = parse_header(body)?;
    let mut index = header.body_offset;

    let (_, count) = read_list_header(body, &mut index)?;
    if count <= HELLO_FIRMWARE_ELEMENT {
        return Err(MessageError::UnexpectedShape {
            what: "hello body list too short for a firmware element",
        });
    }

    for _ in 0..HELLO_FIRMWARE_ELEMENT {
        skip_value(body, &mut index)?;
    }

    let (_, parts) = read_list_header(body, &mut index)?;
    let mut out = String::new();
    for i in 0..parts {
        let part = read_int(body, &mut index)?;
        if i > 0 {
            out.push('.');
        }
        out.push_str(&part.to_string());
    }
    Ok(out)
}

/// Build the frame that switches the pedal to `preset`, verifying it first.
///
/// This is the only write Pinex performs, and it is the one that can break a
/// pedal mid-set, so it is deliberately paranoid:
///
/// 1. Start from the pedal's own most recent state, byte for byte.
/// 2. Change the preset by whichever route the pedal honours — see
///    [`crate::state::PedalState::change_preset`]. In stomp mode that is an
///    in-place write; the documented stage-and-switch is reverted by hardware.
/// 3. Force direct monitoring on, or USB can leave the pedal silent.
/// 4. **Diff the result against the original and refuse to transmit if any byte
///    outside the intended set changed.** Step 4 is the point: it converts "we
///    believe we only touched three bytes" into something checked at runtime,
///    on every write, including against firmware we have never seen.
///
/// The check is a *subset* test, not equality, and real hardware is why. The
/// pedal we captured already had direct monitoring on, so forcing it on changed
/// nothing — three intended offsets, two actual differences. Demanding equality
/// rejected a perfectly safe write. What matters is that nothing unintended
/// moved, never that everything intended did.
///
/// Returns the framed bytes ready for the wire, and the offsets we intended.
pub fn set_preset(current: &PedalState, preset: u8) -> Result<(Vec<u8>, Vec<usize>), MessageError> {
    if preset >= MAX_PRESETS {
        return Err(MessageError::PresetOutOfRange {
            preset,
            max: MAX_PRESETS,
        });
    }

    let mut next = current.clone();
    let intended = next
        .change_preset(preset)
        .map_err(|_| MessageError::UnexpectedShape {
            what: "could not resolve the active slot in the current state",
        })?;

    let actual = crate::state::diff_offsets(current.raw(), next.raw());
    if let Some(&stray) = actual.iter().find(|off| !intended.contains(off)) {
        return Err(MessageError::UnsafeWrite {
            offset: stray,
            intended: intended.clone(),
        });
    }

    Ok((write_state(&next), intended))
}

/// Build a frame applying `edit` to the pedal's own state, verified first.
///
/// The same paranoia as [`set_preset`]: start from the pedal's bytes, apply the
/// smallest possible change, and refuse to transmit if any byte outside the
/// intended set moved. Every stage operation goes through here so none of them
/// can quietly grow into a broader write.
pub fn edit_state<F>(current: &PedalState, edit: F) -> Result<(Vec<u8>, Vec<usize>), MessageError>
where
    F: FnOnce(&mut PedalState) -> Result<Vec<usize>, crate::state::StateError>,
{
    let mut next = current.clone();
    let intended = edit(&mut next).map_err(|_| MessageError::UnexpectedShape {
        what: "state edit could not be applied to this state",
    })?;

    let actual = crate::state::diff_offsets(current.raw(), next.raw());
    if let Some(&stray) = actual.iter().find(|off| !intended.contains(off)) {
        return Err(MessageError::UnsafeWrite {
            offset: stray,
            intended: intended.clone(),
        });
    }
    Ok((write_state(&next), intended))
}

/// A preset's index and display name, as reported by the pedal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetInfo {
    pub index: u8,
    pub name: String,
}

/// Extract the index and name from a preset-details response.
///
/// The body nests as `[1, index, [[name_buffer, name_len], ...]]`. The name
/// buffer is a fixed 33-slot list padded with NULs, and the length that follows
/// it is what says where the name really ends — trusting the buffer alone drags
/// padding into the string.
///
/// Walked structurally rather than by offset, because the preset payload is
/// ~2 KB of parameters whose layout we have not reverse-engineered and should
/// not depend on.
pub fn parse_preset_name(body: &[u8]) -> Result<PresetInfo, MessageError> {
    let header = parse_header(body)?;
    let mut index = header.body_offset;

    let (_, outer) = read_list_header(body, &mut index)?;
    if outer < 3 {
        return Err(MessageError::UnexpectedShape {
            what: "preset body list too short to hold an index and payload",
        });
    }
    // Element 0 is a constant `1` in every capture; element 1 is the index.
    let _ = read_int(body, &mut index)?;
    let preset_index = read_int(body, &mut index)?;

    // Descend to the [name_buffer, name_len] pair.
    read_list_header(body, &mut index)?;
    read_list_header(body, &mut index)?;

    let (_, capacity) = read_list_header(body, &mut index)?;
    let mut chars = Vec::with_capacity(capacity as usize);
    for _ in 0..capacity {
        chars.push(read_int(body, &mut index)?);
    }
    let declared = read_int(body, &mut index)? as usize;

    if declared > chars.len() {
        return Err(MessageError::UnexpectedShape {
            what: "preset name length exceeds its buffer",
        });
    }

    let name = chars[..declared]
        .iter()
        .map(|&c| char::from(c as u8))
        .collect::<String>();

    Ok(PresetInfo {
        index: preset_index as u8,
        name: name.trim_end().to_string(),
    })
}

/// Parse a header without checking the declared size.
///
/// Kept public so a frame that violates the size convention — the expected
/// symptom of a Tonex firmware change — can still be reported with its type and
/// raw bytes instead of vanishing into a generic parse failure.
pub fn parse_header_unvalidated(body: &[u8]) -> Result<Header, MessageError> {
    if body.len() < HEADER_MARKER.len() || body[..2] != HEADER_MARKER {
        return Err(MessageError::BadMarker {
            found: body.iter().take(2).copied().collect(),
        });
    }
    let mut index = 2;
    let type_code = read_int(body, &mut index)?;
    let size = read_int(body, &mut index)?;
    let unknown = read_int(body, &mut index)?;
    Ok(Header {
        msg_type: MessageType::from_code(type_code),
        size,
        unknown,
        body_offset: index,
    })
}

/// Parse a header and require the declared size to match the bytes present.
pub fn parse_header(body: &[u8]) -> Result<Header, MessageError> {
    let header = parse_header_unvalidated(body)?;
    let actual = body.len() - header.body_offset;
    if actual != header.size as usize {
        return Err(MessageError::SizeMismatch {
            declared: header.size,
            actual,
        });
    }
    Ok(header)
}

#[cfg(test)]
mod tests {
    /// The reference implementation's exact template, transcribed once here so
    /// the builder is checked against it rather than against itself.
    ///
    /// `usb_tonex_one_send_master_volume`:
    ///   message {0xb9,0x03,0x81,0x09,0x03,0x82,0x0A,0x00,0x80,0x0B,0x03}
    ///   payload {0xB9,0x04,0x03,0x00,0x00,0x88, f32 }
    #[test]
    fn a_master_volume_write_matches_the_reference_template() {
        let frame = set_master_volume(MAX_MASTER_VOLUME_DB);
        let body = crate::frame::decode_frame(&frame).expect("must be a valid frame");

        assert_eq!(
            &body[..11],
            &[0xb9, 0x03, 0x81, 0x09, 0x03, 0x82, 0x0a, 0x00, 0x80, 0x0b, 0x03],
            "message header"
        );
        assert_eq!(
            &body[11..17],
            &[0xb9, 0x04, 0x03, 0x00, 0x00, 0x88],
            "payload marker"
        );
        assert_eq!(body.len(), 21, "11 header + 10 payload");
    }

    /// The trap. The pedal does **not** take decibels: it takes a 0..10 linear
    /// scale, and the reference converts on both sides. Sending dB straight
    /// through would put -40 on a 0..10 control.
    #[test]
    fn the_wire_value_is_the_zero_to_ten_scale_not_decibels() {
        let wire_of = |db: f32| {
            let frame = set_master_volume(db);
            let body = crate::frame::decode_frame(&frame).unwrap();
            f32::from_le_bytes([body[17], body[18], body[19], body[20]])
        };

        // -40 dB is the bottom of the range, which is 0 on the wire.
        assert!((wire_of(MIN_MASTER_VOLUME_DB) - 0.0).abs() < 0.01);
        // +3 dB is the top, which is 10.
        assert!((wire_of(MAX_MASTER_VOLUME_DB) - 10.0).abs() < 0.01);
        // Unity sits where the reference's formula puts it.
        assert!((wire_of(0.0) - 40.0 / 4.3).abs() < 0.01, "0 dB");
    }

    /// Loudness is the one setting where a bad number is expensive, and the
    /// reference clamps nowhere at all.
    #[test]
    fn master_volume_is_clamped_before_it_reaches_the_pedal() {
        let wire_of = |db: f32| {
            let frame = set_master_volume(db);
            let body = crate::frame::decode_frame(&frame).unwrap();
            f32::from_le_bytes([body[17], body[18], body[19], body[20]])
        };

        assert!(
            (wire_of(1000.0) - 10.0).abs() < 0.01,
            "absurdly loud is capped"
        );
        assert!(
            (wire_of(-1000.0) - 0.0).abs() < 0.01,
            "absurdly quiet is floored"
        );
        assert!(
            (wire_of(f32::NAN) - 0.0).abs() < 0.01,
            "NaN must not reach the pedal"
        );
    }

    /// Round trip through the scale the pedal actually speaks.
    #[test]
    fn decibels_survive_the_round_trip_through_the_wire_scale() {
        for db in [-40.0, -30.0, -12.0, -6.0, 0.0, 3.0] {
            let back = master_volume_from_wire(master_volume_to_wire(db));
            assert!((back - db).abs() < 0.01, "{db} dB came back as {back}");
        }
    }

    #[test]
    fn a_master_volume_request_matches_the_reference_template() {
        let frame = request_master_volume();
        let body = crate::frame::decode_frame(&frame).unwrap();
        assert_eq!(
            body,
            vec![
                0xb9, 0x03, 0x81, 0x0d, 0x03, 0x82, 0x05, 0x00, 0x80, 0x0b, 0x03, 0xb9, 0x03, 0x03,
                0x00, 0x00
            ]
        );
    }

    /// The pedal answers with the same 0x0309 code it accepts, so the reply is
    /// parsed back into decibels — which is what lets the display show what the
    /// pedal actually has rather than what we asked for.
    #[test]
    fn a_param_changed_reply_reports_the_master_volume_in_decibels() {
        // 5.0 on the wire is the midpoint of 0..10.
        let mut body = vec![
            0xb9, 0x03, 0x81, 0x09, 0x03, 0x82, 0x0a, 0x00, 0x80, 0x0b, 0x03, 0xb9, 0x04, 0x03,
            0x00, 0x00, 0x88,
        ];
        body.extend_from_slice(&5.0f32.to_le_bytes());

        let change = parse_param_changed(&body).expect("should parse");
        assert_eq!(change.index, 0, "index 0 is master volume");
        assert!(
            (change.db - (5.0 / 10.0 * 43.0 - 40.0)).abs() < 0.01,
            "got {} dB",
            change.db
        );
    }

    /// A reply about some other parameter must not be mistaken for the volume.
    #[test]
    fn a_reply_about_another_parameter_is_reported_with_its_own_index() {
        let mut body = vec![
            0xb9, 0x03, 0x81, 0x09, 0x03, 0x82, 0x0a, 0x00, 0x80, 0x0b, 0x03, 0xb9, 0x04, 0x03,
            0x07, 0x00, 0x88,
        ];
        body.extend_from_slice(&1.0f32.to_le_bytes());

        let change = parse_param_changed(&body).expect("should parse");
        assert_eq!(change.index, 7);
        assert!(!change.is_master_volume());
    }

    use super::*;
    use crate::frame::decode_frame;
    use crate::state::PedalState;

    /// Payload bytes of the three requests, exactly as both reference
    /// implementations transmit them to real hardware.
    const HELLO_PAYLOAD: &[u8] = &[
        0xb9, 0x03, 0x00, 0x82, 0x04, 0x00, 0x80, 0x0b, 0x01, 0xb9, 0x02, 0x02, 0x0b,
    ];
    const REQUEST_STATE_PAYLOAD: &[u8] = &[
        0xb9, 0x03, 0x00, 0x82, 0x06, 0x00, 0x80, 0x0b, 0x03, 0xb9, 0x02, 0x81, 0x06, 0x03, 0x0b,
    ];

    #[test]
    fn request_payloads_match_the_reference_byte_for_byte() {
        assert_eq!(decode_frame(&hello()).unwrap(), HELLO_PAYLOAD);
        assert_eq!(
            decode_frame(&request_state()).unwrap(),
            REQUEST_STATE_PAYLOAD
        );

        let framed = request_preset(0, PresetDetail::Summary).unwrap();
        assert_eq!(
            decode_frame(&framed).unwrap(),
            vec![
                0xb9, 0x03, 0x81, 0x00, 0x03, 0x82, 0x06, 0x00, 0x80, 0x0b, 0x03, 0xb9, 0x04, 0x0b,
                0x01, 0x00, 0x00
            ]
        );
    }

    #[test]
    fn request_preset_sets_index_and_detail_and_nothing_else() {
        let base = decode_frame(&request_preset(0, PresetDetail::Summary).unwrap()).unwrap();

        for preset in 0..MAX_PRESETS {
            for detail in [PresetDetail::Summary, PresetDetail::Full] {
                let payload = decode_frame(&request_preset(preset, detail).unwrap()).unwrap();
                assert_eq!(payload.len(), base.len());
                assert_eq!(payload[15], preset);
                assert_eq!(payload[16], detail as u8);
                assert_eq!(payload[..15], base[..15], "only bytes 15/16 may vary");
            }
        }
    }

    #[test]
    fn request_preset_rejects_out_of_range_indices() {
        for preset in [MAX_PRESETS, MAX_PRESETS + 1, 255] {
            assert!(matches!(
                request_preset(preset, PresetDetail::Summary),
                Err(MessageError::PresetOutOfRange { .. })
            ));
        }
    }

    #[test]
    fn every_request_is_a_decodable_frame() {
        // Round-tripping through our own codec proves stuffing and CRC agree
        // with each other. It does NOT prove the pedal accepts them; only the
        // byte-for-byte comparison against the reference above speaks to that.
        for framed in [
            hello(),
            request_state(),
            request_preset(19, PresetDetail::Full).unwrap(),
        ] {
            assert!(decode_frame(&framed).is_ok());
        }
    }

    #[test]
    fn write_state_carries_the_raw_body_verbatim() {
        let body: Vec<u8> = (0..40u8).collect();
        let state = PedalState::from_body(body.clone()).unwrap();
        let payload = decode_frame(&write_state(&state)).unwrap();

        assert_eq!(&payload[..2], &[0xb9, 0x03]);
        assert_eq!(&payload[2..5], &[0x81, 0x06, 0x03], "type 0x0306");
        // Declared size is the body length, little-endian.
        assert_eq!(
            u16::from_le_bytes([payload[6], payload[7]]),
            body.len() as u16
        );
        assert_eq!(&payload[11..], &body[..], "body must be echoed unmodified");
    }

    #[test]
    fn parses_a_well_formed_header() {
        // Header for type 0x0306 with a 4-byte body.
        let body = vec![
            0xb9, 0x03, 0x81, 0x06, 0x03, 0x82, 0x04, 0x00, 0x80, 0x0b, 1, 2, 3, 4,
        ];
        let header = parse_header(&body).unwrap();
        assert_eq!(header.msg_type, MessageType::StateUpdate);
        assert_eq!(header.size, 4);
        assert_eq!(header.body_offset, 10);
    }

    #[test]
    fn size_mismatch_is_loud() {
        let body = vec![
            0xb9, 0x03, 0x81, 0x06, 0x03, 0x82, 0x09, 0x00, 0x80, 0x0b, 1, 2,
        ];
        assert!(matches!(
            parse_header(&body),
            Err(MessageError::SizeMismatch {
                declared: 9,
                actual: 2
            })
        ));
        // ...but the header is still recoverable for diagnostics.
        assert_eq!(
            parse_header_unvalidated(&body).unwrap().msg_type,
            MessageType::StateUpdate
        );
    }

    #[test]
    fn bad_marker_is_rejected() {
        assert!(matches!(
            parse_header(&[0x00, 0x01, 0x02]),
            Err(MessageError::BadMarker { .. })
        ));
        assert!(matches!(
            parse_header(&[0xb9]),
            Err(MessageError::BadMarker { .. })
        ));
    }

    #[test]
    fn message_type_codes() {
        assert_eq!(MessageType::from_code(0x02), MessageType::Hello);
        assert_eq!(MessageType::from_code(0x0306), MessageType::StateUpdate);
        assert_eq!(MessageType::from_code(0x1234), MessageType::Unknown(0x1234));
    }

    /// Pins down the `0x80` tag-width discrepancy documented on
    /// [`crate::value::tag_width`].
    ///
    /// Requests carry one structural byte after the `0x80` field that responses
    /// do not (`0x01` for Hello, `0x03` for RequestState and RequestPreset), so
    /// a request's body is `size + 1`. Responses are exactly `size` — see
    /// `tests/fixtures.rs::captured_hello_response_is_internally_consistent`,
    /// which asserts that against real hardware bytes.
    ///
    /// This asymmetry is why the requests once looked like evidence for a 2-byte
    /// `0x80` tag: that reading absorbed the extra byte and made the arithmetic
    /// come out even. See [`crate::value::tag_width`] for what settled it.
    ///
    /// We only ever parse responses, so [`parse_header`] is correct as written.
    #[test]
    fn requests_carry_one_extra_header_byte_that_responses_do_not() {
        let preset_payload =
            decode_frame(&request_preset(0, PresetDetail::Summary).unwrap()).unwrap();

        for payload in [HELLO_PAYLOAD, REQUEST_STATE_PAYLOAD, &preset_payload] {
            let header = parse_header_unvalidated(payload).unwrap();
            let remaining = payload.len() - header.body_offset;
            assert_eq!(
                remaining,
                header.size as usize + 1,
                "request should carry exactly one extra byte: {payload:02x?}"
            );
        }
    }
}
