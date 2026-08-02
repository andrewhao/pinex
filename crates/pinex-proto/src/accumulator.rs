//! Reassembles flag-delimited frames from a byte stream.
//!
//! CDC-ACM reads do not align to frame boundaries: one read can carry half a
//! frame, or two and a half. `vit3k/tonex_controller` handles this by buffering
//! until a closing `0x7E` and flushing a stale partial frame after an inter-byte
//! timeout.
//!
//! This type is pure — it owns no clock. The caller decides when a partial frame
//! has gone stale and calls [`FrameAccumulator::flush_stale`], which keeps timing
//! policy in `pinex-device` and keeps this crate testable without one.

use crate::frame::{FLAG, MIN_FRAME_LEN};

/// Full preset details run ~30 KB; the summary we actually request is ~2 KB.
/// This cap only exists so a desynced stream cannot grow the buffer without
/// bound.
pub const DEFAULT_MAX_FRAME_LEN: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct FrameAccumulator {
    buf: Vec<u8>,
    in_frame: bool,
    max_frame_len: usize,
    /// Bytes dropped through resync or overflow, for diagnostics.
    dropped: usize,
}

impl Default for FrameAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameAccumulator {
    pub fn new() -> Self {
        Self::with_max_frame_len(DEFAULT_MAX_FRAME_LEN)
    }

    pub fn with_max_frame_len(max_frame_len: usize) -> Self {
        Self {
            buf: Vec::new(),
            in_frame: false,
            max_frame_len,
            dropped: 0,
        }
    }

    /// Feed bytes; returns every complete frame that closed, each including both
    /// delimiters so it can go straight to [`crate::decode_frame`].
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        for &b in bytes {
            if !self.in_frame {
                // Outside a frame, everything but an opening flag is noise.
                if b == FLAG {
                    self.in_frame = true;
                    self.buf.clear();
                    self.buf.push(FLAG);
                } else {
                    self.dropped += 1;
                }
                continue;
            }

            self.buf.push(b);

            if b == FLAG {
                if self.buf.len() >= MIN_FRAME_LEN {
                    frames.push(std::mem::take(&mut self.buf));
                    self.in_frame = false;
                } else {
                    // Too short to be a frame: this flag opens rather than
                    // closes. Covers back-to-back `7E 7E` separators.
                    self.dropped += self.buf.len() - 1;
                    self.buf.clear();
                    self.buf.push(FLAG);
                }
                continue;
            }

            if self.buf.len() > self.max_frame_len {
                // Desynced. Drop and wait for the next opening flag.
                self.dropped += self.buf.len();
                self.buf.clear();
                self.in_frame = false;
            }
        }
        frames
    }

    /// Discard any partial frame. Call when the inter-byte timeout expires.
    /// Returns the number of bytes discarded.
    pub fn flush_stale(&mut self) -> usize {
        let n = self.buf.len();
        self.dropped += n;
        self.buf.clear();
        self.in_frame = false;
        n
    }

    /// Bytes currently held in an unterminated frame.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    /// Total bytes discarded as noise, overflow, or stale partials.
    pub fn dropped(&self) -> usize {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::encode_frame;

    #[test]
    fn reassembles_a_frame_fed_one_byte_at_a_time() {
        let framed = encode_frame(b"\xb9\x03\x00\x82\x04\x00");
        let mut acc = FrameAccumulator::new();

        let mut out = Vec::new();
        for &b in &framed {
            out.extend(acc.push(&[b]));
        }

        assert_eq!(out, vec![framed]);
        assert_eq!(acc.pending(), 0);
    }

    #[test]
    fn splits_two_frames_delivered_in_one_chunk() {
        let a = encode_frame(b"first");
        let b = encode_frame(b"second");
        let mut chunk = a.clone();
        chunk.extend_from_slice(&b);

        let mut acc = FrameAccumulator::new();
        assert_eq!(acc.push(&chunk), vec![a, b]);
    }

    #[test]
    fn handles_a_frame_split_across_two_reads() {
        let framed = encode_frame(b"\x00\x01\x02\x03\x04\x05");
        let (head, tail) = framed.split_at(3);

        let mut acc = FrameAccumulator::new();
        assert!(acc.push(head).is_empty());
        assert_ne!(acc.pending(), 0);
        assert_eq!(acc.push(tail), vec![framed]);
    }

    #[test]
    fn resyncs_after_leading_garbage() {
        let framed = encode_frame(b"payload");
        let mut chunk = vec![0x11, 0x22, 0x33];
        chunk.extend_from_slice(&framed);

        let mut acc = FrameAccumulator::new();
        assert_eq!(acc.push(&chunk), vec![framed]);
        assert_eq!(acc.dropped(), 3);
    }

    #[test]
    fn tolerates_back_to_back_flags_between_frames() {
        let a = encode_frame(b"first");
        let b = encode_frame(b"second");
        let mut chunk = a.clone();
        chunk.push(FLAG); // stray separator
        chunk.extend_from_slice(&b);

        let mut acc = FrameAccumulator::new();
        assert_eq!(acc.push(&chunk), vec![a, b]);
    }

    #[test]
    fn flush_stale_discards_a_partial_frame() {
        let framed = encode_frame(b"truncated");
        let mut acc = FrameAccumulator::new();

        assert!(acc.push(&framed[..4]).is_empty());
        assert_eq!(acc.flush_stale(), 4);
        assert_eq!(acc.pending(), 0);

        // A fresh frame still parses after the flush.
        assert_eq!(acc.push(&framed), vec![framed]);
    }

    #[test]
    fn oversized_frame_is_dropped_rather_than_buffered_forever() {
        let mut acc = FrameAccumulator::with_max_frame_len(16);
        let mut junk = vec![FLAG];
        junk.extend(std::iter::repeat_n(0xAA, 64));

        assert!(acc.push(&junk).is_empty());
        assert_eq!(acc.pending(), 0, "buffer must not grow past the cap");

        let framed = encode_frame(b"ok");
        assert_eq!(acc.push(&framed), vec![framed]);
    }
}
