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

/// Header marker every message opens with.
const HEADER_MARKER: [u8; 2] = [0xB9, 0x03];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    Hello,
    StateUpdate,
    PresetResponse,
    /// Acknowledgement of a state write. Carries no payload and no promise.
    WriteAck,
    Unknown(u16),
}

impl MessageType {
    pub fn from_code(code: u16) -> Self {
        match code {
            TYPE_HELLO => Self::Hello,
            TYPE_STATE_UPDATE => Self::StateUpdate,
            TYPE_PRESET_RESPONSE => Self::PresetResponse,
            TYPE_WRITE_ACK => Self::WriteAck,
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
