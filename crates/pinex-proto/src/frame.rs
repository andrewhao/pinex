//! HDLC-style framing: byte stuffing, CRC, and frame encode/decode.
//!
//! Wire format is `0x7E <stuffed payload> <stuffed CRC-16 LE> 0x7E`. The CRC is
//! computed over the *unstuffed* payload and is itself stuffed on the way out.

use crc::{Crc, CRC_16_IBM_SDLC};

/// Frame delimiter.
pub const FLAG: u8 = 0x7E;
/// Escape byte introducing a stuffed octet.
pub const ESCAPE: u8 = 0x7D;
/// Stuffed octets are XORed with this.
pub const ESCAPE_XOR: u8 = 0x20;

/// Smallest legal frame: two flags plus a two-byte CRC over an empty payload.
pub const MIN_FRAME_LEN: usize = 4;

/// The pedal uses CRC-16/IBM-SDLC (a.k.a. X-25): reflected poly `0x1021`
/// (`0x8408` reversed), init `0xFFFF`, xorout `0xFFFF`.
///
/// This is not a guess. `vit3k/tonex_controller`'s `hdlc.cpp` implements the
/// bitwise loop by hand with `0x8408` / init `0xFFFF` / `return ~crc`, which is
/// exactly this parameterisation.
const CRC16: Crc<u16> = Crc::<u16>::new(&CRC_16_IBM_SDLC);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// Too short, or not delimited by `0x7E` at both ends.
    InvalidFrame { len: usize },
    /// An `0x7D` escape was the last byte before the closing flag.
    InvalidEscape { offset: usize },
    /// A bare `0x7E` appeared inside the frame body.
    UnexpectedFlag { offset: usize },
    /// Frame body was too short to contain a CRC.
    Truncated { len: usize },
    /// CRC did not match. Carries both values so failures are attributable.
    CrcMismatch { expected: u16, actual: u16 },
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFrame { len } => {
                write!(f, "not a frame: {len} bytes, missing 0x7E delimiters")
            }
            Self::InvalidEscape { offset } => {
                write!(f, "dangling 0x7D escape at offset {offset}")
            }
            Self::UnexpectedFlag { offset } => {
                write!(f, "unescaped 0x7E inside frame at offset {offset}")
            }
            Self::Truncated { len } => {
                write!(f, "frame body of {len} bytes is too short for a CRC")
            }
            Self::CrcMismatch { expected, actual } => {
                write!(
                    f,
                    "CRC mismatch: frame says {expected:#06x}, computed {actual:#06x}"
                )
            }
        }
    }
}

impl std::error::Error for FrameError {}

/// CRC-16/IBM-SDLC over `data`.
pub fn crc16(data: &[u8]) -> u16 {
    CRC16.checksum(data)
}

/// Append `byte` to `out`, escaping it if it collides with a delimiter.
fn stuff_byte(out: &mut Vec<u8>, byte: u8) {
    if byte == FLAG || byte == ESCAPE {
        out.push(ESCAPE);
        out.push(byte ^ ESCAPE_XOR);
    } else {
        out.push(byte);
    }
}

/// Byte-stuff `payload` without adding flags or a CRC.
pub fn stuff(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len());
    for &b in payload {
        stuff_byte(&mut out, b);
    }
    out
}

/// Reverse [`stuff`]. Rejects dangling escapes and bare flags.
pub fn unstuff(inner: &[u8]) -> Result<Vec<u8>, FrameError> {
    let mut out = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        match inner[i] {
            ESCAPE => {
                let next = inner
                    .get(i + 1)
                    .ok_or(FrameError::InvalidEscape { offset: i })?;
                out.push(next ^ ESCAPE_XOR);
                i += 2;
            }
            FLAG => return Err(FrameError::UnexpectedFlag { offset: i }),
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Wrap `payload` in a complete frame: flags, CRC, and stuffing.
pub fn encode_frame(payload: &[u8]) -> Vec<u8> {
    let crc = crc16(payload);
    let mut out = Vec::with_capacity(payload.len() + MIN_FRAME_LEN);
    out.push(FLAG);
    for &b in payload {
        stuff_byte(&mut out, b);
    }
    // CRC goes out little-endian, and is stuffed like any other byte.
    stuff_byte(&mut out, (crc & 0xFF) as u8);
    stuff_byte(&mut out, (crc >> 8) as u8);
    out.push(FLAG);
    out
}

/// Unwrap a complete flag-delimited frame and verify its CRC.
///
/// `frame` must include both delimiters — that is what [`crate::FrameAccumulator`]
/// emits.
pub fn decode_frame(frame: &[u8]) -> Result<Vec<u8>, FrameError> {
    if frame.len() < MIN_FRAME_LEN || frame[0] != FLAG || frame[frame.len() - 1] != FLAG {
        return Err(FrameError::InvalidFrame { len: frame.len() });
    }

    let mut body = unstuff(&frame[1..frame.len() - 1])?;
    if body.len() < 2 {
        return Err(FrameError::Truncated { len: body.len() });
    }

    let hi = body.pop().expect("length checked above") as u16;
    let lo = body.pop().expect("length checked above") as u16;
    let expected = (hi << 8) | lo;
    let actual = crc16(&body);
    if expected != actual {
        return Err(FrameError::CrcMismatch { expected, actual });
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_reference_parameterisation() {
        // Bitwise reimplementation of vit3k's hdlc.cpp calculateCRC, used as an
        // independent oracle for our table-driven crate.
        fn reference_crc(data: &[u8]) -> u16 {
            let mut crc: u16 = 0xFFFF;
            for &byte in data {
                crc ^= byte as u16;
                for _ in 0..8 {
                    if crc & 1 != 0 {
                        crc = (crc >> 1) ^ 0x8408;
                    } else {
                        crc >>= 1;
                    }
                }
            }
            !crc
        }

        for case in [
            &b""[..],
            &b"\x00"[..],
            &b"\xb9\x03\x00\x82\x04\x00\x80\x0b\x01\xb9\x02\x02\x0b"[..],
            &(0u8..=255).collect::<Vec<_>>()[..],
        ] {
            assert_eq!(crc16(case), reference_crc(case), "case {case:02x?}");
        }
    }

    #[test]
    fn stuffing_round_trips_delimiter_bytes() {
        // The bytes that silently corrupt a frame if stuffing is wrong.
        let payload = vec![0x7E, 0x7D, 0x00, 0x7E, 0x7D, 0x7D, 0xFF, 0x7E];
        let stuffed = stuff(&payload);
        assert!(
            !stuffed[..].contains(&FLAG),
            "stuffed output must not contain a bare flag"
        );
        assert_eq!(unstuff(&stuffed).unwrap(), payload);
    }

    #[test]
    fn frame_round_trips() {
        for payload in [
            vec![],
            vec![0x00],
            vec![0x7E, 0x7D],
            (0u8..=255).collect::<Vec<_>>(),
        ] {
            let framed = encode_frame(&payload);
            assert_eq!(framed[0], FLAG);
            assert_eq!(*framed.last().unwrap(), FLAG);
            assert_eq!(decode_frame(&framed).unwrap(), payload);
        }
    }

    #[test]
    fn crc_bytes_are_stuffed_too() {
        // Search for a payload whose CRC contains a delimiter byte, proving the
        // CRC itself goes through stuffing rather than being appended raw.
        let payload = (0u16..2000)
            .map(|n| n.to_le_bytes().to_vec())
            .find(|p| {
                let c = crc16(p);
                let (lo, hi) = ((c & 0xFF) as u8, (c >> 8) as u8);
                [lo, hi].iter().any(|b| *b == FLAG || *b == ESCAPE)
            })
            .expect("some short payload should produce a delimiter byte in its CRC");

        let framed = encode_frame(&payload);
        assert_eq!(framed.iter().filter(|b| **b == FLAG).count(), 2);
        assert_eq!(decode_frame(&framed).unwrap(), payload);
    }

    #[test]
    fn single_bit_corruption_is_a_crc_mismatch_not_a_parse() {
        let payload = b"\xb9\x03\x00\x82\x06\x00\x80\x0b\x03".to_vec();
        let framed = encode_frame(&payload);
        for i in 1..framed.len() - 1 {
            let mut corrupt = framed.clone();
            corrupt[i] ^= 0x01;
            match decode_frame(&corrupt) {
                Err(FrameError::CrcMismatch { .. })
                | Err(FrameError::InvalidEscape { .. })
                | Err(FrameError::UnexpectedFlag { .. })
                | Err(FrameError::InvalidFrame { .. })
                | Err(FrameError::Truncated { .. }) => {}
                Ok(body) => panic!("corrupting byte {i} still parsed cleanly as {body:02x?}"),
            }
        }
    }

    /// xorshift64*, so the property test is randomised but reproducible — a
    /// failure can be re-run without hunting for the seed.
    struct Rng(u64);

    impl Rng {
        fn next_u64(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn payload(&mut self, max_len: usize) -> Vec<u8> {
            let len = (self.next_u64() as usize) % (max_len + 1);
            (0..len)
                .map(|_| {
                    // Oversample the delimiter bytes; uniform random data almost
                    // never exercises the stuffing path.
                    match self.next_u64() % 4 {
                        0 => FLAG,
                        1 => ESCAPE,
                        _ => self.next_u64() as u8,
                    }
                })
                .collect()
        }
    }

    #[test]
    fn property_stuffing_and_framing_round_trip() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..10_000 {
            let payload = rng.payload(96);

            assert_eq!(
                unstuff(&stuff(&payload)).unwrap(),
                payload,
                "stuff round-trip failed for {payload:02x?}"
            );

            let framed = encode_frame(&payload);
            assert_eq!(
                framed.iter().filter(|b| **b == FLAG).count(),
                2,
                "only the delimiters may be bare flags: {framed:02x?}"
            );
            assert_eq!(
                decode_frame(&framed).unwrap(),
                payload,
                "frame round-trip failed for {payload:02x?}"
            );
        }
    }

    #[test]
    fn rejects_undelimited_input() {
        assert!(matches!(
            decode_frame(&[0x00, 0x01, 0x02, 0x03]),
            Err(FrameError::InvalidFrame { .. })
        ));
        assert!(matches!(
            decode_frame(&[FLAG, 0x00]),
            Err(FrameError::InvalidFrame { .. })
        ));
    }

    #[test]
    fn rejects_dangling_escape() {
        assert!(matches!(
            unstuff(&[0x01, ESCAPE]),
            Err(FrameError::InvalidEscape { offset: 1 })
        ));
    }
}
