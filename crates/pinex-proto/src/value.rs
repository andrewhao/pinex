//! Tagged-value encoding used inside message bodies.
//!
//! Values are self-describing by a leading tag byte:
//!
//! | Tag         | Meaning                                  |
//! |-------------|------------------------------------------|
//! | `0x00-0x7F` | literal small integer (the tag *is* the value) |
//! | `0x80`      | integer, 1 byte follows (values `0x80-0xFF`) |
//! | `0x81`      | u16 little-endian                        |
//! | `0x82`      | u16 little-endian                        |
//! | `0x88`      | IEEE-754 f32 little-endian               |
//! | `0xB9` `0xBA` `0xBC` | list header, followed by an element count |

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueError {
    /// Ran off the end of the buffer mid-value.
    Truncated { offset: usize, need: usize },
    /// Tag byte was not the one the caller required.
    UnexpectedTag {
        offset: usize,
        found: u8,
        expected: &'static str,
    },
}

impl std::fmt::Display for ValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { offset, need } => {
                write!(
                    f,
                    "truncated value at offset {offset}: needed {need} more bytes"
                )
            }
            Self::UnexpectedTag {
                offset,
                found,
                expected,
            } => {
                write!(
                    f,
                    "unexpected tag {found:#04x} at offset {offset}, expected {expected}"
                )
            }
        }
    }
}

impl std::error::Error for ValueError {}

/// How many bytes follow the `0x80` tag: **one**.
///
/// `protocol.md` says `0x80`, `0x81` and `0x82` are all u16 little-endian. Its
/// prose is wrong, and its own captured examples are the proof — a `0xB9 0x03`
/// collection declares three elements, and `0xB9 0x03 0x80 0xFF 0x3F 0x00`
/// only contains three under the 1-byte reading (`0x80 0xFF`=255, `0x3F`=63,
/// `0x00`=0 — an RGB color). Under the 2-byte reading it contains two. Both
/// shipping implementations read it as 1 byte.
///
/// This also answers the question `protocol.md` leaves open — why `0xFF` appears
/// "escaped" with an `0x80` prefix in colors. It is not escaping: bare literals
/// only reach `0x7F`, so `0x3F` fits inline and `0xFF` does not.
///
/// Confirmed against real hardware bytes: the captured Hello response satisfies
/// the strict `remaining == size` check exactly. See
/// `tests/fixtures.rs::captured_hello_response_is_internally_consistent`.
///
/// (Requests are `size + 1` because they carry one extra structural byte that
/// responses lack — not a width problem. See
/// `message::tests::requests_carry_one_extra_header_byte_that_responses_do_not`.)
pub const fn tag_width(tag: u8) -> usize {
    match tag {
        0x80 => 1,
        0x81 | 0x82 => 2,
        _ => 0,
    }
}

/// Read an integer value at `*index`, advancing past it.
///
/// Mirrors `parseValue` in both reference implementations.
pub fn read_int(buf: &[u8], index: &mut usize) -> Result<u16, ValueError> {
    let tag = *buf.get(*index).ok_or(ValueError::Truncated {
        offset: *index,
        need: 1,
    })?;

    match tag {
        0x80..=0x82 => {
            let width = tag_width(tag);
            let start = *index + 1;
            let bytes = buf.get(start..start + width).ok_or(ValueError::Truncated {
                offset: *index,
                need: width + 1,
            })?;
            let value = match width {
                1 => bytes[0] as u16,
                _ => u16::from_le_bytes([bytes[0], bytes[1]]),
            };
            *index += 1 + width;
            Ok(value)
        }
        // A literal small integer encodes itself.
        _ => {
            *index += 1;
            Ok(tag as u16)
        }
    }
}

/// Read an `0x88`-tagged f32 at `*index`, advancing past it.
pub fn read_f32(buf: &[u8], index: &mut usize) -> Result<f32, ValueError> {
    let tag = *buf.get(*index).ok_or(ValueError::Truncated {
        offset: *index,
        need: 1,
    })?;
    if tag != 0x88 {
        return Err(ValueError::UnexpectedTag {
            offset: *index,
            found: tag,
            expected: "0x88 (f32)",
        });
    }
    let start = *index + 1;
    let bytes = buf.get(start..start + 4).ok_or(ValueError::Truncated {
        offset: *index,
        need: 5,
    })?;
    *index += 5;
    Ok(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Read a list header (`0xB9`/`0xBA`/`0xBC`) and its element count.
pub fn read_list_header(buf: &[u8], index: &mut usize) -> Result<(u8, u16), ValueError> {
    let tag = *buf.get(*index).ok_or(ValueError::Truncated {
        offset: *index,
        need: 1,
    })?;
    if !matches!(tag, 0xB9 | 0xBA | 0xBC) {
        return Err(ValueError::UnexpectedTag {
            offset: *index,
            found: tag,
            expected: "0xB9/0xBA/0xBC (list)",
        });
    }
    let mut cursor = *index + 1;
    let count = read_int(buf, &mut cursor)?;
    *index = cursor;
    Ok((tag, count))
}

/// Encode a u16 with the `0x82` tag, the form used for sizes on the wire.
pub fn write_u16(out: &mut Vec<u8>, value: u16) {
    out.push(0x82);
    out.extend_from_slice(&value.to_le_bytes());
}

/// Encode an f32 with the `0x88` tag.
pub fn write_f32(out: &mut Vec<u8>, value: f32) {
    out.push(0x88);
    out.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_small_ints_are_their_own_value() {
        for tag in [0x00u8, 0x01, 0x03, 0x0B, 0x7F] {
            let mut i = 0;
            assert_eq!(read_int(&[tag], &mut i).unwrap(), tag as u16);
            assert_eq!(i, 1, "literal must consume exactly one byte");
        }
    }

    #[test]
    fn tagged_u16s_are_little_endian() {
        for tag in [0x81u8, 0x82] {
            let mut i = 0;
            assert_eq!(read_int(&[tag, 0x06, 0x03], &mut i).unwrap(), 0x0306);
            assert_eq!(i, 3);
        }
    }

    #[test]
    fn tag_0x80_follows_the_reference_implementations() {
        // Both references read one byte after 0x80. If a capture ever proves
        // otherwise, `tag_width` is the single place to change.
        assert_eq!(tag_width(0x80), 1);
        let mut i = 0;
        assert_eq!(read_int(&[0x80, 0x0B, 0x03], &mut i).unwrap(), 0x0B);
        assert_eq!(i, 2, "0x80 consumes tag + 1 byte");
    }

    #[test]
    fn f32_round_trips() {
        for value in [0.0f32, -15.0, 15.0, 0.5] {
            let mut buf = Vec::new();
            write_f32(&mut buf, value);
            let mut i = 0;
            assert_eq!(read_f32(&buf, &mut i).unwrap(), value);
            assert_eq!(i, 5);
        }
    }

    #[test]
    fn u16_round_trips() {
        let mut buf = Vec::new();
        write_u16(&mut buf, 0x0306);
        assert_eq!(buf, vec![0x82, 0x06, 0x03]);
        let mut i = 0;
        assert_eq!(read_int(&buf, &mut i).unwrap(), 0x0306);
    }

    #[test]
    fn list_header_yields_tag_and_count() {
        let mut i = 0;
        assert_eq!(read_list_header(&[0xB9, 0x03], &mut i).unwrap(), (0xB9, 3));
        assert_eq!(i, 2);
    }

    #[test]
    fn truncation_errors_rather_than_panicking() {
        let mut i = 0;
        assert!(matches!(
            read_int(&[], &mut i),
            Err(ValueError::Truncated { .. })
        ));
        let mut i = 0;
        assert!(matches!(
            read_int(&[0x82, 0x01], &mut i),
            Err(ValueError::Truncated { .. })
        ));
        let mut i = 0;
        assert!(matches!(
            read_f32(&[0x88, 0x00], &mut i),
            Err(ValueError::Truncated { .. })
        ));
    }

    #[test]
    fn wrong_tag_is_reported_not_guessed() {
        let mut i = 0;
        assert!(matches!(
            read_f32(&[0x82, 0, 0, 0, 0], &mut i),
            Err(ValueError::UnexpectedTag { found: 0x82, .. })
        ));
        let mut i = 0;
        assert!(matches!(
            read_list_header(&[0x82, 0x01], &mut i),
            Err(ValueError::UnexpectedTag { found: 0x82, .. })
        ));
    }
}
