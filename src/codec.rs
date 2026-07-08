//! The length-prefixed frame codec.
//!
//! Wire format: a 4-byte big-endian `u32` payload length, followed by exactly
//! that many payload bytes. See the crate root docs for the rationale.

use std::fmt;
use std::io;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

/// Number of bytes in the length prefix (a big-endian `u32`).
const LENGTH_PREFIX_LEN: usize = 4;

/// Default maximum payload size: 8 MiB. Frames larger than this are rejected
/// rather than buffered/allocated.
pub const DEFAULT_MAX_FRAME_LEN: usize = 8 * 1024 * 1024;

/// Errors produced while encoding or decoding frames.
#[derive(Debug)]
pub enum FrameError {
    /// A frame declared (or was asked to encode) a payload larger than the
    /// configured maximum. Carries the offending length and the limit.
    FrameTooLarge { len: usize, max: usize },
    /// An error from the underlying transport.
    Io(io::Error),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::FrameTooLarge { len, max } => {
                write!(f, "frame of {len} bytes exceeds maximum of {max} bytes")
            }
            FrameError::Io(e) => write!(f, "i/o error: {e}"),
        }
    }
}

impl std::error::Error for FrameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FrameError::Io(e) => Some(e),
            FrameError::FrameTooLarge { .. } => None,
        }
    }
}

// `Framed` requires the codec error to be constructible from `io::Error`, so
// transport errors flow through without hand-wrapping at every call site.
impl From<io::Error> for FrameError {
    fn from(e: io::Error) -> Self {
        FrameError::Io(e)
    }
}

/// A length-prefixed frame codec with a configurable maximum frame size.
#[derive(Debug, Clone)]
pub struct FrameCodec {
    max_frame_len: usize,
}

impl FrameCodec {
    /// A codec using [`DEFAULT_MAX_FRAME_LEN`].
    pub fn new() -> Self {
        Self {
            max_frame_len: DEFAULT_MAX_FRAME_LEN,
        }
    }

    /// A codec with an explicit maximum payload length.
    ///
    /// The maximum is capped at `u32::MAX` because the wire prefix is a `u32`;
    /// larger values would be unrepresentable on the wire.
    pub fn with_max_frame_len(max_frame_len: usize) -> Self {
        Self {
            max_frame_len: max_frame_len.min(u32::MAX as usize),
        }
    }

    /// The configured maximum payload length, in bytes.
    pub fn max_frame_len(&self) -> usize {
        self.max_frame_len
    }
}

impl Default for FrameCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for FrameCodec {
    type Item = Bytes;
    type Error = FrameError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // --- Partial-frame handling, part 1: the length prefix ---
        // We cannot know the frame size until the full 4-byte prefix has
        // arrived. If it hasn't, reserve room for the missing bytes and return
        // `Ok(None)`. Everything already in `src` stays buffered; Tokio calls
        // us again when the transport delivers more bytes.
        if src.len() < LENGTH_PREFIX_LEN {
            src.reserve(LENGTH_PREFIX_LEN - src.len());
            return Ok(None);
        }

        // Peek at the prefix without consuming it. We only mutate `src` once we
        // know the whole frame is present, so a partial payload leaves the
        // buffer untouched for the next call.
        let mut prefix = [0u8; LENGTH_PREFIX_LEN];
        prefix.copy_from_slice(&src[..LENGTH_PREFIX_LEN]);
        let payload_len = u32::from_be_bytes(prefix) as usize;

        // --- Reject oversized frames before allocating anything ---
        // Because we never `reserve` the payload until it passes this check, a
        // hostile or corrupt prefix cannot make us allocate gigabytes.
        if payload_len > self.max_frame_len {
            return Err(FrameError::FrameTooLarge {
                len: payload_len,
                max: self.max_frame_len,
            });
        }

        let frame_len = LENGTH_PREFIX_LEN + payload_len;

        // --- Partial-frame handling, part 2: the payload ---
        // The prefix is complete but the payload may still be arriving. Reserve
        // capacity for the remainder (so the next read can land in one go) and
        // wait for more bytes.
        if src.len() < frame_len {
            src.reserve(frame_len - src.len());
            return Ok(None);
        }

        // The full frame is buffered. Drop the length prefix, which we no longer
        // need, then hand out the payload.
        src.advance(LENGTH_PREFIX_LEN);

        // --- Zero-copy payload extraction ---
        // `split_to(payload_len)` detaches the first `payload_len` bytes into
        // their own handle that shares the *same* underlying allocation as
        // `src` — a refcounted view, not a `memcpy` into a fresh `Vec`.
        // `freeze()` turns that into an immutable `Bytes`. No per-frame
        // allocation or payload copy happens here; the shared buffer is freed
        // automatically once the last `Bytes` view into it is dropped.
        let payload = src.split_to(payload_len).freeze();
        Ok(Some(payload))
    }
}

impl Encoder<Bytes> for FrameCodec {
    type Error = FrameError;

    fn encode(&mut self, item: Bytes, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let payload_len = item.len();
        if payload_len > self.max_frame_len {
            return Err(FrameError::FrameTooLarge {
                len: payload_len,
                max: self.max_frame_len,
            });
        }

        // Reserve once for prefix + payload to avoid repeated buffer growth.
        dst.reserve(LENGTH_PREFIX_LEN + payload_len);
        // `put_u32` writes big-endian, matching the wire format. The cast is
        // safe: `payload_len <= max_frame_len <= u32::MAX`.
        dst.put_u32(payload_len as u32);
        dst.put_slice(&item);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut codec = FrameCodec::new();
        let payload = Bytes::from_static(b"framed-echo");

        let mut buf = BytesMut::new();
        codec.encode(payload.clone(), &mut buf).unwrap();

        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded, payload);
        // The buffer is fully consumed after one complete frame.
        assert!(buf.is_empty());
    }

    #[test]
    fn partial_frame_yields_only_when_complete() {
        let mut codec = FrameCodec::new();
        let payload = Bytes::from_static(b"drip-fed");

        let mut encoded = BytesMut::new();
        codec.encode(payload.clone(), &mut encoded).unwrap();

        // Feed every byte but the last: the decoder must never yield yet.
        let mut buf = BytesMut::new();
        for &byte in &encoded[..encoded.len() - 1] {
            buf.put_u8(byte);
            assert!(
                codec.decode(&mut buf).unwrap().is_none(),
                "decoder yielded before the frame was complete"
            );
        }

        // The final byte completes the frame — now it decodes exactly once.
        buf.put_u8(encoded[encoded.len() - 1]);
        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded, payload);
        assert!(codec.decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn multiple_frames_in_one_buffer_decode_in_order() {
        let mut codec = FrameCodec::new();
        let payloads = [
            Bytes::from_static(b"one"),
            Bytes::from_static(b"two"),
            Bytes::from_static(b"three"),
        ];

        let mut buf = BytesMut::new();
        for p in &payloads {
            codec.encode(p.clone(), &mut buf).unwrap();
        }

        for expected in &payloads {
            let decoded = codec.decode(&mut buf).unwrap().unwrap();
            assert_eq!(&decoded, expected);
        }
        assert!(codec.decode(&mut buf).unwrap().is_none());
        assert!(buf.is_empty());
    }

    #[test]
    fn empty_payload_round_trips() {
        let mut codec = FrameCodec::new();
        let mut buf = BytesMut::new();
        codec.encode(Bytes::new(), &mut buf).unwrap();
        // A zero-length frame is still a valid frame: a prefix of 0, no payload.
        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn oversized_frame_rejected_on_decode() {
        let mut codec = FrameCodec::with_max_frame_len(8);

        let mut buf = BytesMut::new();
        buf.put_u32(9); // declares a 9-byte payload, over the 8-byte cap
        buf.put_slice(&[0u8; 9]);

        let err = codec.decode(&mut buf).unwrap_err();
        match err {
            FrameError::FrameTooLarge { len, max } => {
                assert_eq!(len, 9);
                assert_eq!(max, 8);
            }
            other => panic!("expected FrameTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn oversized_frame_rejected_on_encode() {
        let mut codec = FrameCodec::with_max_frame_len(4);
        let mut buf = BytesMut::new();
        let err = codec
            .encode(Bytes::from_static(b"too long"), &mut buf)
            .unwrap_err();
        assert!(matches!(err, FrameError::FrameTooLarge { len: 8, max: 4 }));
    }
}
